# Simple Portforward  over libp2p (supports HTTP/HTTPS tunneling)

This example is extension over the original libp2p http-proxy sample to support HTTP/HTTPS tunneling over internet:

![p2pproxy](https://user-images.githubusercontent.com/9625669/198875277-c957ac53-d8f4-4fa7-919c-e0659e6fc9ca.png)


This program tunnels HTTP/HTTPS traffic over libp2p between two machines. First machine is the sharer to which the HTTP/HTTPS traffic is sent from
second machine. First is called as local & second one as the remote here. Local machine will acts as proxy server to the remote machine.
Using this program, you can access your home HTTP servers from a remote machine. You can also access internet using your hosted VPS server.

## Build

From the  directory run the following:

```
> cargo build
```

## Usage

First run the program as follows to the machine where you need to run the proxy server (first machine). 

```sh
cargo run -- --forwarder
```

Then run the program in second machine which will need to use the proxy server in above program. Wait for the machine to show up "Found other peer, proxy active" message before browsing internet over this channel.

```
>cargo run -- --topic <topic-id-from-first-machine>
```

Now you can see the proxy setting of your browser to 127.0.0.1:8080. All the requests will be sent to first machine & it will send back the response.

## Security
Using a randomly generated UUID as a topic name so that it is not easily accesible to others.
