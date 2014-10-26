// The MIT License (MIT)

// Copyright (c) 2014 Y. T. CHUNG

// Permission is hereby granted, free of charge, to any person obtaining a copy of
// this software and associated documentation files (the "Software"), to deal in
// the Software without restriction, including without limitation the rights to
// use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:

// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
// FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
// COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
// IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

//! TcpRelay server that running on the server side

#[phase(plugin, link)]
extern crate log;

use std::sync::{Arc, Mutex};
use std::io::{Listener, TcpListener, Acceptor, TcpStream};
use std::io::{EndOfFile, TimedOut, BrokenPipe};

use config::{Config, SingleServer, MultipleServer};
use relay::Relay;
use relay::socks5::{parse_request_header, SocketAddress, DomainNameAddress};
use relay::tcprelay::cached_dns::CachedDns;

use crypto::cipher;
use crypto::cipher::Cipher;
use crypto::cipher::CipherVariant;

#[deriving(Clone)]
pub struct TcpRelayServer {
    config: Config,
}

impl TcpRelayServer {
    pub fn new(c: Config) -> TcpRelayServer {
        if c.server.is_none() {
            fail!("You have to provide a server configuration");
        } else {
            match c.server.clone().unwrap() {
                SingleServer(..) => (),
                MultipleServer(slist) => {
                    if slist.len() != 1 {
                        fail!("You have to provide exact 1 server configuration");
                    }
                }
            }
        }
        TcpRelayServer {
            config: c,
        }
    }

    fn handle_connect_remote(local_stream: &mut TcpStream, remote_stream: &mut TcpStream,
                                          cipher: &mut CipherVariant) {
        let mut buf = [0u8, .. 0xffff];

        loop {
            match remote_stream.read_at_least(1, buf) {
                Ok(len) => {
                    let real_buf = buf.slice_to(len);

                    let encrypted_msg = cipher.encrypt(real_buf);

                    match local_stream.write(encrypted_msg.as_slice()) {
                        Ok(..) => {},
                        Err(err) => {
                            match err.kind {
                                EndOfFile | TimedOut | BrokenPipe => {},
                                _ => {
                                    error!("Error occurs while writing to local stream: {}", err);
                                }
                            }
                            remote_stream.close_read().unwrap();
                            break
                        }
                    }
                },
                Err(err) => {
                    match err.kind {
                        EndOfFile | TimedOut | BrokenPipe => {},
                        _ => {
                            error!("Error occurs while reading from remote stream: {}", err);
                        }
                    }
                    local_stream.close_write().unwrap();
                    break
                }
            }
        }
    }

    fn handle_connect_local(local_stream: &mut TcpStream, remote_stream: &mut TcpStream,
                            cipher: &mut CipherVariant) {
        let mut buf = [0u8, .. 0xffff];
        loop {
            match local_stream.read(buf) {
                Ok(len) => {
                    let real_buf = buf.slice_to(len);
                    let decrypted_msg = cipher.decrypt(real_buf);
                    remote_stream.write(decrypted_msg.as_slice())
                            .ok().expect("Error occurs while writing to remote stream");
                },
                Err(err) => {
                    match err.kind {
                        EndOfFile | TimedOut | BrokenPipe => {},
                        _ => {
                            error!("Error occurs while reading from client stream: {}", err);
                        }
                    }
                    break
                }
            }
        }
    }
}

impl Relay for TcpRelayServer {
    fn run(&self) {
        let (server_addr, server_port, password, encrypt_method, timeout) = {
                let s = match self.config.clone().server.unwrap() {
                    SingleServer(ref s) => {
                        s.clone()
                    },
                    MultipleServer(slist) => {
                        slist[0].clone()
                    }
                };
                (s.address.to_string(), s.port, Arc::new(s.password.clone()), Arc::new(s.method.clone()), s.timeout)
            };

        let mut acceptor = match TcpListener::bind(server_addr.as_slice(), server_port).listen() {
            Ok(acpt) => acpt,
            Err(e) => {
                fail!("Error occurs while listening server address: {}", e.to_string());
            }
        };

        info!("Shadowsocks listening on {}:{}", server_addr, server_port);

        let dnscache_arc = Arc::new(Mutex::new(CachedDns::new()));

        loop {
            match acceptor.accept() {
                Ok(mut stream) => {
                    stream.set_timeout(timeout);

                    let password = password.clone();
                    let encrypt_method = encrypt_method.clone();
                    let dnscache = dnscache_arc.clone();

                    spawn(proc() {
                        let mut cipher = cipher::with_name(encrypt_method.as_slice(),
                                                       password.as_slice().as_bytes())
                                                .expect("Unsupported cipher");

                        let header = {
                            let mut buf = [0u8, .. 1024];
                            let header_len = stream.read(buf).ok()
                                                    .expect("Error occurs while reading header");
                            let encrypted_header = buf.slice_to(header_len);
                            cipher.decrypt(encrypted_header)
                        };

                        let (_, addr) = match parse_request_header(header.as_slice()) {
                            Ok((header_len, addr)) => (header_len, addr),
                            Err(..) => {
                                fail!("Error occurs while parsing request header, \
                                            maybe wrong crypto method or password");
                            }
                        };
                        info!("Connecting to {}", addr);
                        let mut remote_stream = match addr {
                            SocketAddress(sockaddr) => {
                                match TcpStream::connect(sockaddr.ip.to_string().as_slice(), sockaddr.port) {
                                    Ok(s) => s,
                                    Err(err) => {
                                        fail!("Unable to connect {}: {}", sockaddr, err)
                                    }
                                }
                            },
                            DomainNameAddress(ref domainaddr) => {
                                let ipaddr = match dnscache.lock().resolve(domainaddr.domain_name.as_slice()) {
                                    Some(addr) => addr,
                                    None => {
                                        fail!("Failed to resolve host {}", domainaddr)
                                    }
                                };

                                TcpStream::connect(ipaddr.to_string().as_slice(), domainaddr.port)
                                    .ok().expect(format!("Unable to connect host {}", domainaddr).as_slice())
                            }
                        };

                        let mut remote_local_stream = stream.clone();
                        let mut remote_remote_stream = remote_stream.clone();
                        let mut remote_cipher = cipher.clone();
                        spawn(proc()
                            TcpRelayServer::handle_connect_remote(&mut remote_local_stream,
                                                                  &mut remote_remote_stream,
                                                                  &mut remote_cipher));
                        spawn(proc()
                            TcpRelayServer::handle_connect_local(&mut stream,
                                                                 &mut remote_stream,
                                                                 &mut cipher));
                    });
                },
                Err(e) => {
                    fail!("Error occurs while accepting: {}", e.to_string());
                }
            }
        }
    }
}