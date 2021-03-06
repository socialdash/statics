//! `Controller` is a top layer that handles all http-related
//! stuff like reading bodies, parsing params, forming a response.
//! Basically it provides inputs to `Service` layer and converts outputs
//! of `Service` layer to http responses

pub mod multipart_utils;
pub mod routes;
pub mod utils;

use std::io::Read;
use std::sync::Arc;

use failure;
use failure::Fail;
use futures::future;
use futures::prelude::*;
use hyper;
use hyper::header::{Authorization, Bearer};
use hyper::server::Request;
use hyper::Headers;
use hyper::Post;
use image;
use jsonwebtoken::{decode, Algorithm, Validation};
use multipart::server::Multipart;

use stq_http::client::ClientHandle;
use stq_http::controller::{Controller, ControllerFuture};
use stq_http::errors::ErrorMessageWrapper;
use stq_http::request_util::serialize_future;
use stq_router::RouteParser;

use self::routes::Route;
use config::Config;
use errors::*;
use sentry_integration::log_and_capture_error;
use services::s3::S3;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JWTPayload {
    pub user_id: i32,
    pub exp: i64,
}

pub fn verify_token(jwt_key: Vec<u8>, leeway: i64, headers: &Headers) -> Box<Future<Item = JWTPayload, Error = failure::Error>> {
    Box::new(
        future::result(
            headers
                .get::<Authorization<Bearer>>()
                .map(|auth| auth.clone())
                .ok_or_else(|| format_err!("Missing token").context(Error::Unauthorized).into()),
        )
        .and_then(move |auth| {
            let token = auth.0.token.as_ref();

            let validation = Validation {
                leeway,
                ..Validation::new(Algorithm::RS256)
            };
            decode::<JWTPayload>(token, &jwt_key, &validation)
                .map_err(|e| format_err!("Failed to parse JWT token: {}", e).context(Error::Unauthorized).into())
        })
        .map(|t| t.claims),
    )
}

/// Controller handles route parsing and calling `Service` layer
pub struct ControllerImpl {
    pub config: Config,
    pub jwt_public_key: Vec<u8>,
    pub route_parser: Arc<RouteParser<Route>>,
    pub client: ClientHandle,
    pub s3: Arc<S3>,
}

impl ControllerImpl {
    /// Create a new controller based on services
    pub fn new(config: Config, jwt_public_key: Vec<u8>, client: ClientHandle, s3: Arc<S3>) -> Self {
        let route_parser = Arc::new(routes::create_route_parser());
        Self {
            config,
            jwt_public_key,
            route_parser,
            client,
            s3,
        }
    }
}

impl Controller for ControllerImpl {
    /// Handle a request and get future response
    fn call(&self, req: Request) -> ControllerFuture {
        let s3 = self.s3.clone();

        let fut = match (req.method(), self.route_parser.test(req.path())) {
            // POST /images
            (&Post, Some(Route::Images)) => serialize_future({
                let method = req.method().clone();
                let headers = req.headers().clone();

                info!("Received image upload request");

                future::ok(())
                    .and_then({
                        let headers = headers.clone();
                        let leeway = self.config.jwt.leeway;
                        let jwt_key = self.jwt_public_key.clone();
                        move |_| verify_token(jwt_key, leeway, &headers)
                    })
                    .and_then(|_user_id| {
                        read_bytes(req.body()).map_err(|e| e.context("Failed to read request body").context(Error::Network).into())
                    })
                    .and_then(move |bytes| {
                        info!("Read payload bytes");
                        let multipart_wrapper = multipart_utils::MultipartRequest::new(method, headers, bytes);
                        Multipart::from_request(multipart_wrapper).map_err(|_| {
                            format_err!("Couldn't convert request body to multipart")
                                .context(Error::Parse)
                                .into()
                        })
                    })
                    .and_then(|mut multipart_entity| {
                        let mut files: Vec<Vec<u8>> = Vec::new();
                        multipart_entity
                            .foreach_entry(|mut field| {
                                let mut file_data: Vec<u8> = Vec::new();
                                let _ = field.data.read_to_end(&mut file_data);
                                files.push(file_data);
                            })
                            .map_err(|e| format_err!("Parsed multipart, could not iterate over entries: {}", e).context(Error::Parse))?;
                        Ok(files)
                    })
                    .map(futures::stream::iter_ok)
                    .flatten_stream()
                    .and_then(|file| {
                        image::guess_format(&file)
                            .map_err(|e| e.context("Invalid image format").context(Error::Image).into())
                            .map(|format| (format, file))
                            .into_future()
                    })
                    .and_then(move |(format, data)| {
                        Box::new(
                            s3.upload_image(format, data)
                                .map(|name| json!({ "url": name }))
                                .map_err(|e| e.context(Error::Image).into()),
                        )
                    })
                    .collect()
                    .and_then(|uploaded_images| {
                        if uploaded_images.len() == 1 {
                            uploaded_images.into_iter().next().ok_or(format_err!("No images were sent"))
                        } else {
                            serde_json::to_value(&uploaded_images).map_err(|e| {
                                format_err!("Uploaded images, could not serialize result: {}", e)
                                    .context(Error::Parse)
                                    .into()
                            })
                        }
                    })
            }),

            // Fallback
            _ => serialize_future::<String, _, _>(Err(Error::NotFound)),
        }
        .map_err(|err| {
            let wrapper = ErrorMessageWrapper::<Error>::from(&err);
            if wrapper.inner.code == 500 {
                log_and_capture_error(&err);
            }
            err
        });

        Box::new(fut)
    }
}

/// Reads body of request and response in Future format
pub fn read_bytes(body: hyper::Body) -> Box<Future<Item = Vec<u8>, Error = hyper::Error>> {
    Box::new(body.fold(Vec::new(), |mut acc, chunk| {
        acc.extend_from_slice(&*chunk);
        future::ok::<_, hyper::Error>(acc)
    }))
}
