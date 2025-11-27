var uuid;

var connections = [];
var candidates = [];
var socket = null;
var first_offer = "";

window.addEventListener("load", function(){
    connect();
});
function connect() {
  createCall("default")
}

//===========================STREAM=================================

function initCalls() {
    console.log("Initiating calls...");
    const cameras = document.getElementsByClassName("camera");
    for (var i = 0; i < cameras.length; i++) {
        const identifier = cameras[i].getAttribute("identifier");
        createCall(identifier);
    }
}

function createCall(identifier) {
    console.log("Calling: " + identifier);
    const configuration = {'iceServers': [{'urls': 'stun:stun.l.google.com:19302'}]}
    connections[identifier] = new RTCPeerConnection(configuration);
    
    connections[identifier].addTransceiver('audio', { direction: 'recvonly' })
    connections[identifier].addTransceiver('video', { direction: 'recvonly' })

    connections[identifier].ontrack = (event) => {
        if (getVideoElement(identifier).srcObject !== event.streams[0]) {
            console.log("Incoming stream");
            getVideoElement(identifier).srcObject = event.streams[0];
            getVideoElement(identifier).addEventListener("playing", () => {
                console.log("playing");
            });

        }
    };
    connections[identifier].onicecandidate = (event) => {
        if (event.candidate == null) {
            console.log("ICE Candidate was null, done");
            connections[identifier].onicecandidate = null;
        } else {
            send_ice_candidate(event.candidate);
        }
    };
    start_gathering(identifier).then(() => {
        send_sdp_offer(identifier);
    });
}

async function send_ice_candidate(candidate) {
    if (socket) {
        socket.send(JSON.stringify(candidate.toJSON()));
    } else {
        console.log("socket null, waiting...");
        setTimeout(() => {
            send_ice_candidate(candidate);
        }, 1000);
    }
}

async function start_gathering(identifier) {
    return connections[identifier].createOffer().then(async offer => {
        first_offer = offer.sdp;
        return connections[identifier].setLocalDescription(offer);
    });
}

async function send_sdp_offer(identifier) {
    return fetch("/api/whep", {
        headers: {
             Accept: "application/sdp",
            "Content-Type": "application/sdp",
            Authorization: "Bearer " + identifier,
            "User-Agent": "whip-rs"
        },
        method: "POST",
        body: connections[identifier].localDescription.sdp,
    }).then((res) => {
        handle_websocket(identifier, res.headers.get("location"));
        return res.text();
    }).then(async (answer) => {
        console.log("Answer:");
        console.log(answer);
        return connections[identifier].setRemoteDescription({
            type: "answer",
            sdp: answer
        });
    });
}

function getVideoElement() {
    return document.querySelector("video");
}

function handle_websocket(identifier, location) {
    let ws_protocol = "wss";
    if (window.location.protocol == "http:") {
        ws_protocol = "ws";
    }
    ws_url = ws_protocol + "://" + window.location.host + location;
    console.log("new websocket: " + ws_url);
    socket = new WebSocket(ws_url);
    socket.onopen = function(e) {
        console.log("[open] Connection established");
        console.log("Sending to server");
    };

    socket.onmessage = function(event) {
        console.log(`[message] Data received from server: ${event.data}`);

        let candidate = JSON.parse(event.data);
        connections[identifier].addIceCandidate(candidate);
    };

    socket.onclose = function(event) {
        if (event.wasClean) {
            console.log(`[close] Connection closed cleanly, code=${event.code} reason=${event.reason}`);
      } else {
        // par exemple : processus serveur arrêté ou réseau en panne
        // event.code est généralement 1006 dans ce cas
        console.log('[close] Connection died');
      }
    };

    socket.onerror = function(error) {
        console.log(`[error]`);
    };
}
