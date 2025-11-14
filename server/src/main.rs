use std::{path::PathBuf, sync::Mutex};

use actix_cors::Cors;
use actix_files as fs;
use actix_web::{
    App, HttpServer, Responder,
    web::{self, Data},
};

#[derive(Clone, Debug)]
struct WhipData {
    sdp_offers: Vec<String>,
}

async fn sdp_offer(offer: String, whip_data: Data<Mutex<WhipData>>) -> impl Responder {
    let mut whip_data = whip_data.lock().unwrap();
    println!("sdp_offer");
    dbg!(offer.clone());
    whip_data.sdp_offers.push(offer);
    dbg!(whip_data.sdp_offers.len());
    "{sdp answer}"
}

async fn sdp_offers(whip_data: Data<Mutex<WhipData>>) -> impl Responder {
    let whip_data = whip_data.lock().unwrap();
    println!("sdp_offers");
    dbg!(whip_data.sdp_offers.len());
    dbg!(whip_data.sdp_offers.clone());
    let last_offer = whip_data.sdp_offers.last().unwrap().clone();
    return last_offer;
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let whip_data = Data::new(Mutex::new(WhipData { sdp_offers: vec![] }));

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allowed_methods(["POST"]) // https://www.ietf.org/archive/id/draft-ietf-wish-whip-01.html#section-4-7
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(Data::clone(&whip_data.clone()))
            .service(
                web::scope("/api")
                    .route("/whip", web::post().to(sdp_offer))
                    .route("/whep", web::post().to(sdp_offers)),
            )
            .service(
                fs::Files::new("", "./static")
                    .show_files_listing()
                    .use_last_modified(true),
            )
    })
    .bind(("127.0.0.1", 8080))?
    .run()
    .await
}
