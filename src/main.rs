#[macro_use]
extern crate log;
extern crate env_logger;
extern crate argparse;
extern crate futures;
extern crate tokio;
extern crate rand;
extern crate dns_lookup;

use rand::Rng;
use std::env;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::sync::{Arc, Mutex};
use std::process::exit;
use dns_lookup::lookup_host;
use futures::{Future, Stream};
use tokio::io::{copy, shutdown};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

use argparse::{ArgumentParser, Store, Collect};

fn run_proxy() -> Result<(), Box<std::error::Error>> {

    let mut urls: Vec<std::net::SocketAddr> = Vec::new();
    let mut servers: Vec<String> = Vec::new();
    let mut bind = "127.0.0.1:8000".to_string();
    let mut log_level = "info".to_string();

    {
        let mut ap = ArgumentParser::new();
        ap.set_description("TCP load balancer");

        ap.refer(&mut servers).add_argument("server", Collect, "Servers to load balance");

        ap.refer(&mut bind).add_option(&["-b", "--bind"],
                                      Store,
                                      "Bind the load balancer to address:port (127.0.0.1:8000)");

        ap.refer(&mut log_level).add_option(&["-l", "--log"],
                                            Store,
                                            "Log level [debug, info, warn, error] (info)");

        ap.parse_args_or_exit();
    }

    env::set_var("RUST_LOG", log_level);
    env_logger::init();

    if servers.is_empty() {
        error!("Need at least one server to load balance");
        exit(1);
    }
    for url in servers {
      if !url.contains(":") {
        error!("You need to specify loadbalanced server port on {}. It should look like: host:80", url);
        exit(1);
      }
      let parts: Vec<&str> = url.split(":").collect();
      for mut host in lookup_host(parts[0]).unwrap() {
        let sock_addr = std::net::SocketAddr::new(host, parts[1].parse::<u16>().unwrap());
        debug!("{} was resolved to {}", url, sock_addr);
        urls.push(sock_addr);
      }
    }

    let addr = bind.parse().unwrap();
    let sock = TcpListener::bind(&addr).unwrap();
    info!("TCP load balancer listening on {}", bind);
    info!("forwarding traffic to");
    for url in urls.clone() {
      info!(" *  {}", url);
    }
    info!(" ");

    let done = sock
        .incoming()
        .map_err(|e| println!("Error accepting socket; error = {:?}", e))
        .for_each(move |client| {
            let index = rand::thread_rng().gen_range(0, urls.len());
            let url = &urls[index];
            let server = TcpStream::connect(&url);
            let amounts = server.and_then(move |server| {
                let client_reader = MyTcpStream(Arc::new(Mutex::new(client)));
                let client_writer = client_reader.clone();
                let server_reader = MyTcpStream(Arc::new(Mutex::new(server)));
                let server_writer = server_reader.clone();
                let client_to_server = copy(client_reader, server_writer)
                    .and_then(|(n, _, server_writer)| shutdown(server_writer).map(move |_| n));

                let server_to_client = copy(server_reader, client_writer)
                    .and_then(|(n, _, client_writer)| shutdown(client_writer).map(move |_| n));

                client_to_server.join(server_to_client)
            });

            let msg = amounts
                .map(move |(from_client, from_server)| {
                    debug!(
                        "client wrote {} bytes and received {} bytes",
                        from_client, from_server
                    );
                })
                .map_err(|e| {
                    debug!("Error in amounts, don't panic maybe the client just disconnected too soon: {}", e);
                });

            tokio::spawn(msg);

            Ok(())
        });

    tokio::run(done);
    Ok(())
}

#[derive(Clone)]
struct MyTcpStream(Arc<Mutex<TcpStream>>);

impl Read for MyTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.lock().unwrap().read(buf)
    }
}

impl Write for MyTcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl AsyncRead for MyTcpStream {}

impl AsyncWrite for MyTcpStream {
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        try!(self.0.lock().unwrap().shutdown(Shutdown::Write));
        Ok(().into())
    }
}

fn main() -> Result<(), Box<std::error::Error>> {
    run_proxy()
}