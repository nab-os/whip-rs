use std::{collections::HashMap, env, sync::Arc, time::Duration};

use argh::{FromArgs, from_env};

use actix_cors::Cors;
use actix_files as fs;
use actix_web::{
    App, HttpMessage, HttpRequest, HttpResponse, HttpServer, Responder, ResponseError, delete,
    http::{StatusCode, header::ContentType},
    middleware, options, patch, post,
    web::{self, Data, Path},
};
use actix_web_httpauth::extractors::bearer::BearerAuth;

use tokio::{net::UdpSocket, sync::Mutex};
use webrtc::{
    api::{
        API, APIBuilder,
        interceptor_registry::register_default_interceptors,
        media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine},
        setting_engine::SettingEngine,
    },
    ice::{
        udp_mux::{UDPMuxDefault, UDPMuxParams},
        udp_network::UDPNetwork,
    },
    ice_transport::{ice_candidate_type::RTCIceCandidateType, ice_server::RTCIceServer},
    interceptor::registry::Registry,
    peer_connection::{
        RTCPeerConnection, configuration::RTCConfiguration,
        sdp::session_description::RTCSessionDescription,
    },
    rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication,
    rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTPCodecType},
    track::{
        track_local::{TrackLocal, TrackLocalWriter, track_local_static_rtp::TrackLocalStaticRTP},
        track_remote::TrackRemote,
    },
};

use uuid::Uuid;

/// Whip signaling broadcast server
#[derive(FromArgs)]
struct Args {
    /// an optional port to setup the web server
    #[argh(option, short = 'p')]
    port: Option<u16>,

    /// an optional port to setup udp muxing
    #[argh(option, short = 'u')]
    udp_mux_port: Option<u16>,

    /// an optional list of ips separated by '|' to setup nat 1 to 1
    #[argh(option, short = 'i')]
    nat_ips: Option<String>,
}

#[derive(Clone)]
struct WhipData {
    api: Arc<API>,
    default_config: RTCConfiguration,
    whips: Arc<Mutex<HashMap<Uuid, Arc<RTCPeerConnection>>>>,
    subscriptions:
        Arc<Mutex<HashMap<String, Vec<(Arc<TrackLocalStaticRTP>, Arc<TrackLocalStaticRTP>)>>>>,
}

type Result<T> = std::result::Result<T, Error>;
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    SessionGetError(#[from] actix_session::SessionGetError),
    #[error("{0}")]
    SessionInsertError(#[from] actix_session::SessionInsertError),

    #[error("Bad UUID: {0}")]
    BadUuid(#[from] uuid::Error),

    #[error("Webrtc Error: {0}")]
    WebrtcError(#[from] webrtc::Error),

    #[error("Internal Error: {0}")]
    InternalError(String),
}

impl ResponseError for Error {
    fn status_code(&self) -> actix_web::http::StatusCode {
        eprintln!("{}", self);
        match self {
            Error::SessionGetError(_) => StatusCode::SERVICE_UNAVAILABLE,
            Error::SessionInsertError(_) => StatusCode::SERVICE_UNAVAILABLE,
            Error::BadUuid(_) => StatusCode::BAD_REQUEST,
            Error::WebrtcError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code())
            .insert_header(ContentType::html())
            .body(self.to_string())
    }
}

#[options("/whip")]
async fn whip_options(whip_data: Data<WhipData>) -> Result<impl Responder> {
    let mut res = HttpResponse::Ok();
    res.content_type("application/sdp");

    // Headers
    for ice_server in whip_data.default_config.ice_servers.iter() {
        for url in ice_server.urls.iter() {
            res.insert_header(("Link", format!("<{url}>; rel=\"ice-server\";")));
        }
    }

    Ok(res)
}

#[post("/whip")]
async fn whip(
    auth: BearerAuth,
    offer: String,
    whip_data: Data<WhipData>,
) -> Result<impl Responder> {
    let session_id = Uuid::new_v4();
    println!("New whip session: {session_id}");
    let stream_key = auth.token().to_string();
    let pc = Arc::new(
        whip_data
            .api
            .new_peer_connection(whip_data.default_config.clone())
            .await?,
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
                        };
                    }
                } else {
                    subscriptions.insert(sk, Vec::new());
                }
            }
        });
        Box::pin(async move {})
    }));

    pc.set_remote_description(RTCSessionDescription::offer(offer)?)
        .await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;

    pc.gathering_complete_promise().await.recv().await;

    whip_data.whips.lock().await.insert(session_id, pc.clone());

    let late_answer = pc.local_description().await.unwrap().sdp;

    let mut res = HttpResponse::Created();
    res.content_type("application/sdp");

    // Headers
    res.insert_header(("Location", format!("/api/resource/{session_id}")));
    for ice_server in whip_data.default_config.ice_servers.iter() {
        for url in ice_server.urls.iter() {
            res.insert_header(("Link", format!("<{url}>; rel=\"ice-server\";")));
        }
    }

    Ok(res.body(late_answer))
}

#[delete("/resource/{session_id}")]
async fn whip_delete(
    auth: BearerAuth,
    session_id: Path<String>,
    whip_data: Data<WhipData>,
) -> Result<impl Responder> {
    let session_id = Uuid::parse_str(&session_id)?;
    let stream_key = auth.token().to_string();
    let whips = whip_data.whips.lock().await;
    let pc = whips.get(&session_id).unwrap();
    pc.close().await?;

    let mut subscriptions = whip_data.subscriptions.lock().await;
    subscriptions.remove(&stream_key);
    Ok(HttpResponse::Ok())
}

enum ExpectedFields {
    NONE,
    N(usize),
    MANY,
}

fn extract_sdp_field<'a>(
    lines: Vec<&'a str>,
    prefix: &'a str,
    expects: ExpectedFields,
) -> Result<Vec<&'a str>> {
    let fields: Vec<&str> = lines
        .into_iter()
        .filter_map(|line| line.strip_prefix(prefix))
        .collect();

    match expects {
        ExpectedFields::NONE => {
            if fields.len() != 0 {
                return Err(Error::InternalError("SDP malformed".to_string()));
            }
        }
        ExpectedFields::N(n) => {
            if fields.len() != n {
                return Err(Error::InternalError("SDP malformed".to_string()));
            }
        }
        ExpectedFields::MANY => {
            if fields.len() == 0 {
                return Err(Error::InternalError("SDP malformed".to_string()));
            }
        }
    }
    Ok(fields)
}

#[patch("/resource/{session_id}")]
async fn whip_patch(
    req: HttpRequest,
    session_id: Path<String>,
    sdp_patch: String,
    whip_data: Data<WhipData>,
) -> Result<impl Responder> {
    if req.content_type() != "application/trickle-ice-sdpfrag" {
        return Ok(HttpResponse::BadRequest());
    }

    let session_id = Uuid::parse_str(&session_id)?;
    let patch_lines: Vec<&str> = sdp_patch.split("\r\n").collect();

    let patch_ufrags =
        extract_sdp_field(patch_lines.clone(), "a=ice-ufrag:", ExpectedFields::N(1))?;
    let patch_ufrag = patch_ufrags.last().unwrap();
    let patch_pwds = extract_sdp_field(patch_lines.clone(), "a=ice-pwd:", ExpectedFields::N(1))?;
    let patch_pwd = patch_pwds.last().unwrap();

    let whips = whip_data.whips.lock().await;
    let pc = whips.get(&session_id).unwrap();

    let remote_description = pc.remote_description().await.unwrap().sdp;
    let description_lines: Vec<&str> = remote_description.split("\r\n").collect();

    let current_ufrags = extract_sdp_field(
        description_lines.clone(),
        "a=ice-ufrag:",
        ExpectedFields::MANY,
    )?;
    let current_ufrag = current_ufrags.first().unwrap();
    let current_pwds = extract_sdp_field(
        description_lines.clone(),
        "a=ice-pwd:",
        ExpectedFields::MANY,
    )?;
    let current_pwd = current_pwds.first().unwrap();

    if current_ufrag == patch_ufrag && current_pwd == patch_pwd {
        let candidates: Vec<&str> = patch_lines
            .clone()
            .into_iter()
            .filter(|line| line.starts_with("a=candidate"))
            .filter_map(|candidate| candidate.strip_prefix("a="))
            .collect();

        for candidate in candidates {
            pc.add_ice_candidate(webrtc::ice_transport::ice_candidate::RTCIceCandidateInit {
                candidate: candidate.to_string(),
                sdp_mid: None,
                sdp_mline_index: None,
                username_fragment: None,
            })
            .await?;
        }
    } else {
        pc.restart_ice().await?;
    }

    let mut res = HttpResponse::Created();
    res.content_type("application/trickle-ice-sdpfrag");
    Ok(HttpResponse::NoContent())
}

#[post("/whep")]
async fn whep(
    auth: BearerAuth,
    offer: String,
    whip_data: Data<WhipData>,
) -> Result<impl Responder> {
    let session_id = Uuid::new_v4();
    println!("New whep session: {session_id}");
    let stream_key = auth.token().to_string();
    let pc = Arc::new(
        whip_data
            .api
            .new_peer_connection(whip_data.default_config.clone())
            .await?,
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
        .await?;

    let rtp_sender_audio = pc
        .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await?;

    tokio::spawn(async move {
        let mut rtcp_buf = vec![0u8; 1500];
        while let Ok((_, _)) = rtp_sender_video.read(&mut rtcp_buf).await {}
    });
    tokio::spawn(async move {
        let mut rtcp_buf = vec![0u8; 1500];
        while let Ok((_, _)) = rtp_sender_audio.read(&mut rtcp_buf).await {}
    });

    let mut subscriptions = whip_data.subscriptions.lock().await;
    if let Some(subscribers) = subscriptions.get_mut(&stream_key) {
        subscribers.push((video_track, audio_track));
    } else {
        subscriptions.insert(stream_key, Vec::new());
    }

    pc.set_remote_description(RTCSessionDescription::offer(offer)?)
        .await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;

    pc.gathering_complete_promise().await.recv().await;

    let mut whips = whip_data.whips.lock().await;
    whips.insert(session_id, pc.clone());

    let late_answer = pc.local_description().await.unwrap().sdp;

    let mut res = HttpResponse::Created();
    res.content_type("application/sdp");

    // Headers
    res.insert_header(("Location", format!("/api/resource/{session_id}")));
    for ice_server in whip_data.default_config.ice_servers.iter() {
        for url in ice_server.urls.iter() {
            res.insert_header(("Link", format!("<{url}>; rel=\"ice-server\";")));
        }
    }

    Ok(res.body(late_answer))
}

async fn not_found(req: HttpRequest) -> impl Responder {
    dbg!(req);
    HttpResponse::NotFound()
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args: Args = from_env();

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

    // Settings
    let mut setting_engine = SettingEngine::default();

    let mut web_port: Option<u16> = match env::var("PORT").ok() {
        Some(port) => port.parse::<u16>().ok(),
        None => None,
    };
    if let Some(port) = args.port {
        web_port = Some(port);
    }
    let web_port = web_port.unwrap_or(8080);

    let mut udp_mux_port: Option<u16> = match env::var("UDP_MUX_PORT").ok() {
        Some(port) => port.parse::<u16>().ok(),
        None => None,
    };
    if let Some(udp_port) = args.udp_mux_port {
        udp_mux_port = Some(udp_port);
    }
    if let Some(udp_port) = udp_mux_port {
        println!("Using UDP MUX port: {}", udp_port);
        let udp_socket = UdpSocket::bind(("0.0.0.0", udp_port)).await.unwrap();
        let udp_mux = UDPMuxDefault::new(UDPMuxParams::new(udp_socket));
        setting_engine.set_udp_network(UDPNetwork::Muxed(udp_mux));
    }

    let mut nat_ips: Option<String> = env::var("NAT_IPS").ok();
    if let Some(ips) = args.nat_ips
        && !ips.is_empty()
    {
        nat_ips = Some(ips);
    }
    if let Some(nat_ips) = nat_ips {
        let nat_ips: Vec<String> = nat_ips
            .split(',')
            .map(|ip| ip.to_string())
            .filter(|ip| !ip.is_empty())
            .collect();
        if !nat_ips.is_empty() {
            println!("Using NAT 1 to 1 with IPs:");
            for ip in nat_ips.clone().into_iter() {
                println!(" - {}", ip);
            }
            setting_engine.set_nat_1to1_ips(nat_ips, RTCIceCandidateType::Host);
        }
    }

    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m).unwrap();
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting_engine)
        .build();

    let whip_data = Data::new(WhipData {
        api: Arc::new(api),
        default_config: config,
        whips: Arc::new(Mutex::new(HashMap::new())),
        subscriptions: Arc::new(Mutex::new(HashMap::new())),
    });

    println!("Listening on 0.0.0.0:{web_port}");
    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(["POST", "DELETE", "PATCH"])
            .allow_any_header();

        App::new()
            .wrap(cors)
            .wrap(middleware::DefaultHeaders::new().add(("Permissions-Policy", "autoplay=(self)")))
            .app_data(Data::clone(&whip_data.clone()))
            .service(
                web::scope("/api")
                    .service(whip_options)
                    .service(whip)
                    .service(whip_patch)
                    .service(whep)
                    .service(whip_delete),
            )
            .service(
                fs::Files::new("", "./static")
                    .show_files_listing()
                    .index_file("index.html")
                    .use_last_modified(true),
            )
            .default_service(web::to(not_found))
    })
    .bind(("0.0.0.0", web_port))?
    .run()
    .await
}
