# Omniroom
Simple whip broadcast server and client made with Rust  
[A french note I wrote to explain what I plan with this project](https://nablog.fr/realisations/omniroom/0002/)

## Performances
Latency with OBS as virtual webcam with only a pipewiresrc source and piped in gstreamer with gst-launch: 80ms
Latency with the same but piped through encoder and decoder (x264) on the same machine: 140ms
Latency by using Omniroom: 200ms

Latency around 200ms with [meetecho/simple-whip-client](https://github.com/meetecho/simple-whip-client) as a client OBS as a virtual webcam on a local network.  

## Workspace architecture
The project contains:  
- a server part that broadcasts a WHIP stream from a producer to consumers (it also serves the web client)
- a client that takes a video device (ex: /dev/video0) as source and streams it over WHIP
  
## Todo
- Trickle ICE  
- Client for better IP Handling and video handling (maybe even lowering latency even more)  
- Web UI  
- Unit tests  
- Chat with data channel (maybe ?)  
