#[macro_use]
extern crate log;
extern crate env_logger;
extern crate argparse;
extern crate futures;
extern crate tokio_core;
extern crate rand;
extern crate dns_lookup;

use dns_lookup::{lookup_host, lookup_addr};
use rand::Rng;
use std::env;
use std::process::exit;
use futures::{Future, Stream};
use tokio_core::io::{copy, Io};
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;

use argparse::{ArgumentParser, Store, Collect};

fn main() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();

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

    let addr = bind.parse().unwrap();
    let sock = TcpListener::bind(&addr, &handle).unwrap();

    info!("Load balancer listening on {}", bind);

    for url in servers {
        let parts: Vec<&str> = url.split(":").collect();
        debug!("{} was resolved to: ", parts[0]);
        for mut host in lookup_host(parts[0]).unwrap() {

            let sockAddr = std::net::SocketAddr::new(host, parts[1].parse::<u16>().unwrap());
            // host.set_port(parts[1].parse().unwrap());
            info!("{} was resolved to {}", url, sockAddr);
            urls.push(sockAddr);
        }
    }

    let server = sock.incoming().for_each(|(client_stream, remote_addr)| {
        
        let index = rand::thread_rng().gen_range(0, urls.len());

        let (client_read, client_write) = client_stream.split();

        debug!("{} connected and will be forwarded to {}", &remote_addr, &urls[index]);

        let send_data = TcpStream::connect(&urls[index], &handle).and_then(|server_stream| {
            let (server_read, server_write) = server_stream.split();

            let client_to_server = copy(client_read, server_write);
            let server_to_client = copy(server_read, client_write);

            client_to_server.join(server_to_client)
        })
        // erase the types
            .map(|(_client_to_server,_server_to_client)| {} ).map_err(|_err| {} );

        handle.spawn(send_data);

        Ok(())
    });

    core.run(server).unwrap();
}