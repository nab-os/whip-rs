use std::{env, sync::Arc};
use webrtc::{
    api::media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS},
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{TrackLocal, track_local_static_rtp::TrackLocalStaticRTP},
};

use argh::FromArgs;

use tokio::net::UdpSocket;
use webrtc::{
    api::{
        APIBuilder, interceptor_registry::register_default_interceptors, media_engine::MediaEngine,
        setting_engine::SettingEngine,
    },
    ice::{
        udp_mux::{UDPMuxDefault, UDPMuxParams},
        udp_network::UDPNetwork,
    },
    ice_transport::{ice_candidate_type::RTCIceCandidateType, ice_server::RTCIceServer},
    interceptor::registry::Registry,
    peer_connection::configuration::RTCConfiguration,
    track::track_remote::TrackRemote,
};

/// Whip signaling broadcast server
#[derive(FromArgs)]
struct Args {
    /// an optional port to setup udp muxing
    #[argh(option, short = 'u')]
    udp_mux_port: Option<u16>,

    /// an optional list of ips separated by '|' to setup nat 1 to 1
    #[argh(option, short = 'i')]
    nat_ips: Option<String>,
}

type Result<T> = std::result::Result<T, Error>;
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Webrtc Error: {0}")]
    WebrtcError(#[from] webrtc::Error),

    #[error("Internal Error: {0}")]
    InternalError(String),
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Args = argh::from_env();

    let default_config = RTCConfiguration {
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
    if let Some(ips) = args.nat_ips {
        nat_ips = Some(ips);
    }
    if let Some(nat_ips) = nat_ips {
        let nat_ips: Vec<String> = nat_ips.split(',').map(|ip| ip.to_string()).collect();
        println!("Using NAT 1 to 1 with IPs:");
        for ip in nat_ips.clone().into_iter() {
            println!(" - {}", ip);
        }
        setting_engine.set_nat_1to1_ips(nat_ips, RTCIceCandidateType::Host);
    }

    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m).unwrap();
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting_engine)
        .build();

    let pc = Arc::new(api.new_peer_connection(default_config).await?);

    let video_track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            ..Default::default()
        },
        format!("video"),
        format!("omniroom-client"),
    ));
    let audio_track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            ..Default::default()
        },
        format!("audio"),
        format!("omniroom-client"),
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

    // let pc2 = pc.clone();
    pc.on_track(Box::new(move |_track: Arc<TrackRemote>, _, _| {
        // tokio::spawn(async move {
        //     while let Ok((rtp, _)) = track.read_rtp().await {
        //         let sk = sk.clone();
        //         let mut subscriptions = subscribers_lock.lock().await;
        //         let subscribers = subscriptions.get(&sk);
        //         if let Some(subscribers) = subscribers {
        //             for (video_track, audio_track) in subscribers {
        //                 match track.kind() {
        //                     RTPCodecType::Video => {
        //                         video_track.write_rtp(&rtp).await.unwrap();
        //                     }
        //                     RTPCodecType::Audio => {
        //                         audio_track.write_rtp(&rtp).await.unwrap();
        //                     }
        //                     RTPCodecType::Unspecified => {}
        //                 };
        //             }
        //         } else {
        //             subscriptions.insert(sk, Vec::new());
        //         }
        //     }
        // });
        Box::pin(async move {})
    }));

    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer.clone()).await?;

    pc.gathering_complete_promise().await.recv().await;

    let late_offer = pc.local_description().await.unwrap().sdp;
    println!("{}", late_offer);
    // reqwest POST offer then:
    // pc.set_remote_description(RTCSessionDescription::offer(offer)?)
    //     .await?;

    Ok(())
}
