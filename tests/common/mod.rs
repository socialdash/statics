extern crate rand;

use statics_lib;

use std::thread;
use std::time;
use hyper::Client;
use hyper::client::HttpConnector;
use tokio_core::reactor::Core;
use std::sync::mpsc::channel;
use self::rand::Rng;

type HttpClient = Client<HttpConnector>;

pub struct Context {
    pub client: HttpClient,
    pub base_url: String,
    pub core: Core,
}

pub fn setup() -> Context {
    let (tx, rx) = channel::<bool>();
    let mut rng = rand::thread_rng();
    let port = rng.gen_range(50000, 60000);
    thread::spawn(move || {
        let config = statics_lib::config::Config::new().expect("Can't load app config!");
        statics_lib::start_server(config, Some(port.to_string()), move || { tx.send(true); });
    });
    rx.recv().unwrap();
    let core = Core::new().expect("Unexpected error creating event loop core");
    let client = Client::new(&core.handle());
    Context {
        client,
        base_url: format!("http://localhost:{}", port),
        core,
    }
}
