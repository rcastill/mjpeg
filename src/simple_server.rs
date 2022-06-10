use std::net::SocketAddr;

use futures::{Future, FutureExt, TryStreamExt};
use http::{
    header::{InvalidHeaderValue, CONTENT_LENGTH, CONTENT_TYPE},
    HeaderMap, HeaderValue, Request, Response, StatusCode,
};
use hyper::{
    service::{make_service_fn, service_fn},
    Body,
};

use crate::MjpegHandle;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] http::Error),
    #[error("mjpeg error: {0}")]
    Mjpeg(#[from] crate::Error),
    // wanted to do <HeaderValue as FromStr>::Err:
    // that went wrong; it complained with conflicting implementation with
    // impl From<T> for T. Found an issue:
    // https://github.com/dtolnay/thiserror/issues/156; seems to be a dead
    // end. Nothing that cannot be fixed by explicitly defining the error
    // type
    #[error("header error: {0}")]
    Header(#[from] InvalidHeaderValue),
    #[error("hyper error: {0}")]
    Hyper(#[from] hyper::Error),
}

#[inline]
fn status_code_response(
    status: http::StatusCode,
) -> Result<Response<Body>, Error> {
    let body = Body::from(status.to_string());
    Response::builder()
        .status(status)
        .body(body)
        .map_err(Error::from)
}

async fn serve(
    req: Request<Body>,
    handle: MjpegHandle,
) -> Result<Response<Body>, Error> {
    // extract streamid
    // http://host/streamid
    let path = req.uri().path();
    let mut split = path.split("/");
    let streamid_opt = split.nth(1).and_then(|streamid| {
        if split.next().is_some() {
            return None;
        }
        Some(streamid)
    });
    let streamid = match streamid_opt {
        Some(streamid) => streamid,
        None => return status_code_response(StatusCode::BAD_REQUEST),
    };

    // get stream "streamid" if it exists
    let stream = match handle.stream(streamid).await {
        Ok(stream) => stream,
        Err(crate::Error::StreamNotFound) => {
            return status_code_response(StatusCode::NOT_FOUND)
        }
        Err(e) => {
            log::error!("Error while trying to extract stream: {e}");
            return status_code_response(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // common headers
    let image_jpeg: HeaderValue = "image/jpeg".parse()?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, image_jpeg);

    // generate stream with jpegs
    let stream = stream.map_ok(move |body| {
        let len = body.len();

        // add Content-Length
        let mut headers = headers.clone();
        let content_length: HeaderValue = format!("{len}")
            .parse()
            // this should never fail
            .expect("Failed parsing header length");
        headers.insert(CONTENT_LENGTH, content_length);

        let part = multipart_stream::Part { headers, body };
        part
    });

    // serialize multipart stream with boundary MJPEGFRAME
    let stream = multipart_stream::serialize(stream, "MJPEGFRAME");

    Ok(hyper::Response::builder()
        .header(
            http::header::CONTENT_TYPE,
            "multipart/x-mixed-replace; boundary=MJPEGFRAME",
        )
        .body(hyper::Body::wrap_stream(stream))?)
}

pub fn run<A>(
    handle: MjpegHandle,
    addr: A,
    graceful_shutdown: impl Future<Output = ()>,
) -> impl Future<Output = Result<(), Error>>
where
    A: Into<SocketAddr>,
{
    let make_svc = make_service_fn(move |_conn| {
        let handle = handle.clone();
        futures::future::ok::<_, std::convert::Infallible>(service_fn(
            move |req| {
                let handle = handle.clone();
                serve(req, handle)
            },
        ))
    });

    // Start server
    // TODO: stop server if primary handle is dropped
    hyper::Server::bind(&addr.into())
        .serve(make_svc)
        .with_graceful_shutdown(graceful_shutdown)
        .map(|res| res.map_err(Error::from))
    // log::info!("Serving on http://{}", server.local_addr());
    // server.await.map_err(Error::from) // from hyper::Error
}
