# Omniroom
Simple whip broadcast server and client made with Rust

## Performances
Latency around 20ms with [meetecho/simple-whip-client](https://github.com/meetecho/simple-whip-client) as a client OBS as a virtual webcam.  

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
