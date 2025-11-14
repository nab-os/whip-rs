use dioxus::prelude::*;

// const FAVICON: Asset = asset!("/assets/favicon.ico");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        // document::Link { rel: "icon", href: FAVICON }
        // document::Link { rel: "stylesheet", href: MAIN_CSS }
        Player {}
    }
}

#[component]
pub fn Player() -> Element {
    rsx! {
        video {
            id: "whip_player",
        }
    }
}
