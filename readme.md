# TCP load balancer

## Build

```sh
$ cargo build --release
```

## Usage

```sh
$ tcp-lb -h

Usage:
    tcp-lb [OPTIONS] [SERVER ...]

TCP load balancer

positional arguments:
  server                Servers to load balance

optional arguments:
  -h,--help             show this help message and exit
  -b,--bind BIND        Bind the load balancer to address:port (127.0.0.1:8000)
  -l,--log LOG          Log level [debug, info, warn, error] (info)

$ tcp-lb neverssl.com:80 -l error
$ curl -H 'Host: neverssl.com' http://127.0.0.1:8000
```