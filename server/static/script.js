var uuid;

var connections = [];
var first_offer = "";

var socket;
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
            send_sdp_offer(identifier);
            connections[identifier].onicecandidate = null;
        }
    };
    start_gathering(identifier);
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
            "User-Agent": "omniroom"
        },
        method: "POST",
        body: connections[identifier].localDescription.sdp,
    }).then((res) => {
        return res.text();
    }).then(async (answer) => {
        return connections[identifier].setRemoteDescription({
            type: "answer",
            sdp: answer
        });
    });
}

function getVideoElement() {
    return document.querySelector("video");
}
