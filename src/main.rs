use std::{collections::HashMap, sync::Arc, time::Duration};

use thiserror::Error;

use actix_cors::Cors;
use actix_files as fs;
use actix_web::{
    App, HttpRequest, HttpResponse, HttpServer, Responder, delete, get, middleware, post, rt,
    web::{self, Data, Path},
};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use actix_ws::AggregatedMessage;
use futures_util::StreamExt as _;

use tokio::sync::{
    Mutex,
    mpsc::{self, Receiver, Sender, channel},
};
use webrtc::{
    api::{
        API, APIBuilder,
        interceptor_registry::register_default_interceptors,
        media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine},
    },
    ice_transport::{
        ice_candidate::{RTCIceCandidate, RTCIceCandidateInit},
        ice_server::RTCIceServer,
    },
    interceptor::registry::Registry,
    peer_connection::{
        RTCPeerConnection, configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        policy::ice_transport_policy::RTCIceTransportPolicy,
        sdp::session_description::RTCSessionDescription, signaling_state::RTCSignalingState,
    },
    rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication,
    rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTPCodecType},
    track::{
        track_local::{TrackLocal, TrackLocalWriter, track_local_static_rtp::TrackLocalStaticRTP},
        track_remote::TrackRemote,
    },
};

use uuid::Uuid;

mod websocket;

#[derive(Clone)]
struct WhipData {
    api: Arc<API>,
    default_config: RTCConfiguration,
    whips: Arc<Mutex<HashMap<Uuid, Arc<RTCPeerConnection>>>>,
    channels: Arc<Mutex<HashMap<Uuid, Receiver<RTCIceCandidate>>>>,
    subscriptions:
        Arc<Mutex<HashMap<String, Vec<(Arc<TrackLocalStaticRTP>, Arc<TrackLocalStaticRTP>)>>>>,
}

#[derive(Error, Debug)]
pub enum WhipRsError {
    #[error("{0}")]
    SessionGetError(#[from] actix_session::SessionGetError),
    #[error("{0}")]
    SessionInsertError(#[from] actix_session::SessionInsertError),

    #[error("Bad UUID: {0}")]
    BadUuid(#[from] uuid::Error),
}

#[post("/whip")]
async fn whip(auth: BearerAuth, offer: String, whip_data: Data<WhipData>) -> impl Responder {
    let session_id = Uuid::new_v4();
    println!("whip: {session_id}");
    let stream_key = auth.token().to_string();

    let (tx, rx) = channel::<RTCIceCandidate>(100);

    let pc = Arc::new(
        whip_data
            .api
            .new_peer_connection(whip_data.default_config.clone())
            .await
            .unwrap(),
    );

    let wd = whip_data.clone();
    let sk = stream_key.clone();
    let pc2 = pc.clone();
    pc.on_track(Box::new(move |track: Arc<TrackRemote>, _, _| {
        // RTCP
        let media_ssrc = track.ssrc();
        let pc2 = pc2.clone();
        tokio::spawn(async move {
            let pc2 = pc2.clone();
            let mut result = webrtc::error::Result::<usize>::Ok(0);
            while result.is_ok() {
                let pc2 = pc2.clone();
                let timeout = tokio::time::sleep(Duration::from_secs(3));
                tokio::pin!(timeout);

                tokio::select! {
                _ = timeout.as_mut() =>{
                        result = pc2.write_rtcp(&[Box::new(PictureLossIndication{
                            sender_ssrc: 0,
                            media_ssrc,
                        })]).await.map_err(Into::into);
                    }
                }
            }
        });

        let subscribers_lock = wd.subscriptions.clone();
        let sk = sk.clone();
        tokio::spawn(async move {
            println!("enter track loop {}", track.kind());
            while let Ok((rtp, _)) = track.read_rtp().await {
                let sk = sk.clone();
                let mut subscriptions = subscribers_lock.lock().await;
                let subscribers = subscriptions.get(&sk);
                if let Some(subscribers) = subscribers {
                    for (video_track, audio_track) in subscribers {
                        match track.kind() {
                            RTPCodecType::Video => {
                                video_track.write_rtp(&rtp).await.unwrap();
                            }
                            RTPCodecType::Audio => {
                                audio_track.write_rtp(&rtp).await.unwrap();
                            }
                            RTPCodecType::Unspecified => {}
                        }
                    }
                } else {
                    subscriptions.insert(sk, Vec::new());
                }
            }
            println!("exit track loop {}", track.kind());
        });
        Box::pin(async move {})
    }));

    pc.on_signaling_state_change(Box::new(move |s: RTCSignalingState| {
        println!("Signaling State has changed: {s}");
        Box::pin(async {})
    }));

    pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        println!("Peer Connection State has changed: {s}");

        if s == RTCPeerConnectionState::Failed {
            // Wait until PeerConnection has had no network activity for 30 seconds or another failure. It may be reconnected using an ICE Restart.
            // Use webrtc.PeerConnectionStateDisconnected if you are interested in detecting faster timeout.
            // Note that the PeerConnection may come back from PeerConnectionStateDisconnected.
            println!("Peer Connection has gone to failed exiting");
        }

        Box::pin(async {})
    }));

    dbg!(offer.clone());

    pc.set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .await
        .unwrap();
    let answer = pc.create_answer(None).await.unwrap();
    pc.set_local_description(answer.clone()).await.unwrap();

    pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
        if let Some(candidate) = c {
            dbg!(candidate.to_string());
            tx.try_send(candidate).unwrap();
        }
        Box::pin(async {})
    }));

    // pc.gathering_complete_promise().await.recv().await;

    dbg!(pc.local_description().await.unwrap().sdp);

    whip_data.whips.lock().await.insert(session_id, pc.clone());
    whip_data.channels.lock().await.insert(session_id, rx);

    HttpResponse::Created()
        .content_type("application/sdp")
        .insert_header(("Location", format!("/api/resource/{session_id}")))
        .body(pc.local_description().await.unwrap().sdp)
}

#[delete("/resource/{session_id}")]
async fn whip_stop(
    auth: BearerAuth,
    session_id: Path<String>,
    whip_data: Data<WhipData>,
) -> impl Responder {
    println!("DELETE: {}", session_id);
    let session_id = Uuid::parse_str(&session_id).unwrap();
    let stream_key = auth.token().to_string();
    let whips = whip_data.whips.lock().await;
    let pc = whips.get(&session_id).unwrap();
    pc.close().await.unwrap();

    let mut subscriptions = whip_data.subscriptions.lock().await;
    subscriptions.remove(&stream_key);
    HttpResponse::Ok()
}

#[post("/whep")]
async fn whep(auth: BearerAuth, offer: String, whip_data: Data<WhipData>) -> impl Responder {
    let session_id = Uuid::new_v4();
    println!("whep: {session_id}");
    let stream_key = auth.token().to_string();

    let (tx, rx) = channel::<RTCIceCandidate>(100);

    let pc = Arc::new(
        whip_data
            .api
            .new_peer_connection(whip_data.default_config.clone())
            .await
            .unwrap(),
    );

    let video_track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            ..Default::default()
        },
        format!("video_{session_id}"),
        format!("webrtc-rs_{session_id}"),
    ));
    let audio_track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            ..Default::default()
        },
        format!("audio_{session_id}"),
        format!("webrtc-rs_{session_id}"),
    ));

    let rtp_sender_video = pc
        .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .unwrap();

    let rtp_sender_audio = pc
        .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .unwrap();

    // tokio::spawn(async move {
    //     let mut rtcp_buf = vec![0u8; 1500];
    //     while let Ok((_, _)) = rtp_sender_video.read(&mut rtcp_buf).await {}
    // });
    // tokio::spawn(async move {
    //     let mut rtcp_buf = vec![0u8; 1500];
    //     while let Ok((_, _)) = rtp_sender_audio.read(&mut rtcp_buf).await {}
    // });

    let mut subscriptions = whip_data.subscriptions.lock().await;
    if let Some(subscribers) = subscriptions.get_mut(&stream_key) {
        subscribers.push((video_track, audio_track));
    } else {
        subscriptions.insert(stream_key, Vec::new());
    }

    pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
        if let Some(candidate) = c {
            dbg!(candidate.to_string());
            tx.try_send(candidate).unwrap();
        }
        Box::pin(async {})
    }));

    pc.set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .await
        .unwrap();
    let answer = pc.create_answer(None).await.unwrap();
    pc.set_local_description(answer.clone()).await.unwrap();

    // pc.gathering_complete_promise().await.recv().await;

    let mut whips = whip_data.whips.lock().await;
    whips.insert(session_id, pc);
    whip_data.channels.lock().await.insert(session_id, rx);

    HttpResponse::Created()
        .content_type("application/sdp")
        .insert_header(("Location", format!("/api/resource/{session_id}")))
        .body(answer.sdp)
}

async fn not_found(req: HttpRequest) -> impl Responder {
    dbg!(req);
    HttpResponse::NotFound()
}

#[get("/resource/{session_id}")]
async fn signaling(
    session_id: Path<String>,
    whip_data: Data<WhipData>,
    req: HttpRequest,
    stream: web::Payload,
) -> impl Responder {
    println!("New websocket");
    let (res, mut session, stream) = actix_ws::handle(&req, stream).unwrap();

    let mut stream = stream
        .aggregate_continuations()
        // aggregate continuation frames up to 1MiB
        .max_continuation_size(2_usize.pow(20));

    let session_id = Uuid::parse_str(&session_id).unwrap();

    let wd = whip_data.clone();
    rt::spawn(async move {
        let mut channels = wd.channels.lock().await;
        let rx = channels.get_mut(&session_id).unwrap();
        while let Some(candidate) = rx.recv().await {
            let candidate = candidate.to_json().unwrap();
            session
                .text(serde_json::to_string(&candidate).unwrap())
                .await
                .unwrap();
        }
    });

    // start task but don't wait for it
    let wd = whip_data.clone();
    rt::spawn(async move {
        // receive messages from websocket
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(AggregatedMessage::Text(text)) => {
                    let candidate: RTCIceCandidateInit = serde_json::from_str(&text).unwrap();
                    let mut whips = wd.whips.lock().await;
                    let pc = whips.get_mut(&session_id).unwrap();
                    pc.add_ice_candidate(candidate).await.unwrap();
                }

                Ok(AggregatedMessage::Binary(bin)) => {
                    // echo binary message
                }

                Ok(AggregatedMessage::Ping(msg)) => {
                    // respond to PING frame with PONG frame
                }

                _ => {}
            }
        }
    });

    // respond immediately with response connected to WS session
    res
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    // Api build
    let mut m = MediaEngine::default();
    m.register_default_codecs().unwrap();
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m).unwrap();
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    let whip_data = Data::new(WhipData {
        api: Arc::new(api),
        default_config: config,
        whips: Arc::new(Mutex::new(HashMap::new())),
        channels: Arc::new(Mutex::new(HashMap::new())),
        subscriptions: Arc::new(Mutex::new(HashMap::new())),
    });

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(["POST", "DELETE"])
            .allow_any_header();

        App::new()
            .wrap(cors)
            .wrap(middleware::DefaultHeaders::new().add(("Permissions-Policy", "autoplay=(self)")))
            .app_data(Data::clone(&whip_data.clone()))
            .service(
                web::scope("/api")
                    .service(whip)
                    .service(whep)
                    .service(whip_stop)
                    .service(signaling),
            )
            .service(
                fs::Files::new("", "./static")
                    .show_files_listing()
                    .index_file("index.html")
                    .use_last_modified(true),
            )
            .default_service(web::to(not_found))
    })
    .bind(("0.0.0.0", 8080))
    .unwrap()
    .run()
    .await
}
