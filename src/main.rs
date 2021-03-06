//! Statics is a microservice responsible for uploading different static assets like images, videos, etc.
//! The layered structure of the app is
//!
//! `Application -> Controller -> Service -> HttpClient`
//!
//! Currently available routes:
//!
//! - `GET /healthcheck` - returns `"ok"` if the server is live
//! - `POST /images` - accepts multipart HTTP requests with `png` / `jpeg` images.
//! Returns `{"url": <url of uploaded image>}`. You can also use prefix with this url
//! to get different sizes: thumb - 40 pixels, small - 80 pixels, medium - 320 pixels,
//! large - 640 pixels. Example: `https://s3.amazonaws.com/storiqa-dev/img-2IpSsAjuxB8C.png` is original image,
//! `https://s3.amazonaws.com/storiqa-dev/img-2IpSsAjuxB8C-large.png` is large image.

extern crate statics_lib as lib;
extern crate stq_logging;

fn main() {
    let config = lib::Config::new().expect("Failed to load service configuration. Please check your 'config' folder");

    // Prepare sentry integration
    let _sentry = lib::sentry_integration::init(config.sentry.as_ref());

    // Prepare logger
    stq_logging::init(config.graylog.as_ref());

    lib::start_server(config, None, || ());
}
