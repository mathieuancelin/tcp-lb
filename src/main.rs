#[macro_use]
extern crate log;
extern crate env_logger;
extern crate argparse;
extern crate futures;
extern crate tokio;
extern crate rand;
extern crate net2;
extern crate dns_lookup;

use argparse::{ArgumentParser, Store, Collect};
use dns_lookup::lookup_host;
use futures::{Future, Stream};
use rand::Rng;
use std::env;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr};
use std::sync::{Arc, Mutex};
use std::thread;
use std::process::exit;
use tokio::io::{copy, shutdown};
use tokio::net::{TcpListener, TcpStream};
use tokio::reactor::Handle;
use tokio::prelude::*;

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

#[cfg(unix)]
fn configure_tcp(tcp: &net2::TcpBuilder) -> io::Result<()> {
    use net2::unix::*;
    try!(tcp.reuse_port(true));
    Ok(())
}

fn listener(addr: &SocketAddr) -> io::Result<TcpListener> {
    let listener = match *addr {
        SocketAddr::V4(_) => try!(net2::TcpBuilder::new_v4()),
        SocketAddr::V6(_) => try!(net2::TcpBuilder::new_v6()),
    };
    try!(configure_tcp(&listener));
    try!(listener.reuse_address(true));
    try!(listener.bind(addr));
    listener.listen(1024).and_then(|l| {
        TcpListener::from_std(l, &Handle::default())
    })
}

fn serve(addr: SocketAddr, urls: Vec<std::net::SocketAddr>, w: u16) -> io::Result<()> {
  let worker = w.clone();
  let sock = listener(&addr)?;
  let done = sock
    .incoming()
    .map_err(move |e| error!("[worker-{}] Error accepting socket; error = {:?}", worker, e))
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
                    "[worker-{}] client wrote {} bytes and received {} bytes",
                    worker, from_client, from_server
                );
            })
            .map_err(move |e| {
                debug!("[worker-{}] Error in amounts, don't panic maybe the client just disconnected too soon: {}", worker, e);
            });

        tokio::spawn(msg);

        Ok(())
    });
  tokio::run(done);
  Ok(())
}

fn run_proxy() -> Result<(), Box<std::error::Error>> {

    let mut urls: Vec<std::net::SocketAddr> = Vec::new();
    let mut servers: Vec<String> = Vec::new();
    let mut bind = "127.0.0.1:8000".to_string();
    let mut log_level = "info".to_string();
    let mut workers = 1;

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
                                      
        ap.refer(&mut workers).add_option(&["-w", "--workers"],
                                            Store,
                                            "Number of workers");

        ap.parse_args_or_exit();
    }

    env::set_var("RUST_LOG", log_level);
    env_logger::init();

    if servers.is_empty() {
        error!("You need to specify at least one server to load balance");
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

    let addr: SocketAddr = bind.parse().unwrap();
    // let sock = TcpListener::bind(&addr).unwrap();
    info!("TCP load balancer listening on {}", bind);
    info!("forwarding traffic to");
    for url in urls.clone() {
      info!(" *  {}", url);
    }
    let threads = (0..workers).map(|i| {
        info!("Starting worker {} ...", i);
        let the_urls = urls.clone();
        let the_addr = addr.clone();
        thread::Builder::new().name(format!("worker-{}", i)).spawn(move || {
            serve(the_addr, the_urls, i);
        }).unwrap()
    }).collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    Ok(())
}

fn main() -> Result<(), Box<std::error::Error>> {
    run_proxy()
}