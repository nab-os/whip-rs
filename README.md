# Omniroom
Simple whip broadcast server made with Rust

## Performances
Latency around 20ms with [meetecho/simple-whip-client](https://github.com/meetecho/simple-whip-client) as a client OBS as a virtual webcam.  
  
## Installation
```
git clone https://github.com/nab-os/omniroom
cd omniroom
cargo build
```

### Docker compose
Install docker & docker compose  
Just copy the `compose.yml` file from the repository  
```
sudo docker compose up -d
```
  
## Usage
```
Usage: omniroom [-p <port>] [-u <udp-mux-port>] [-i <nat-ips>]

Whip signaling broadcast server

Options:
  -p, --port        an optional port to setup the web server
  -u, --udp-mux-port
                    an optional port to setup udp muxing
  -i, --nat-ips     an optional list of ips separated by '|' to setup nat 1 to 1
  --help, help      display usage information

```
  
## Todo
- Trickle ICE  
- Client for better IP Handling and video handling (maybe even lowering latency even more)  
- Web UI  
- Chat with data channel (maybe ?)  
