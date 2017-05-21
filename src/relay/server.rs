//! Server side

use std::io;

use tokio_core::reactor::Core;

use futures::Future;

use relay::udprelay::server::run as run_udp;
use relay::tcprelay::server::run as run_tcp;
use config::Config;
use relay::Context;

/// Relay server running on server side.
///
/// ```no_run
/// use shadowsocks::relay::server::run;
/// use shadowsocks::config::{Config, ServerConfig};
/// use shadowsocks::crypto::CipherType;
///
/// let mut config = Config::new();
/// config.server = vec![
///     ServerConfig::basic("127.0.0.1:8388".parse().unwrap(),
///                         "server-password".to_string(),
///                         CipherType::Aes256Cfb)];
/// run(config).unwrap();
/// ```
///
pub fn run(config: Config) -> io::Result<()> {
    let mut lp = try!(Core::new());
    let handle = lp.handle();

    // let config = Rc::new(config);

    // let tcp_fut = run_tcp(config.clone(), handle.clone());

    // if config.enable_udp {
    //     lp.run(tcp_fut.join(run_udp(config, handle)).map(|_| ()))
    // } else {
    //     lp.run(tcp_fut)
    // }

    let enable_udp = config.enable_udp;
    let context = Context::new(handle, config);
    Context::set(&context, move || if enable_udp {
        let tcp_fut = run_tcp();
        let udp_fut = run_udp();
        lp.run(tcp_fut.join(udp_fut).map(|_| ()))
    } else {
        let tcp_fut = run_tcp();
        lp.run(tcp_fut)
    })
}
