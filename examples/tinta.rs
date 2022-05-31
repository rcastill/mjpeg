// use warp::Filter;

// async fn stream(_id: String) -> &'static str {
//     "uff"
// }

use std::convert::Infallible;
use std::time::Duration;

use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Request, Response};
use hyper::service::{make_service_fn, service_fn};
use hyper::Body;
use tokio::io::AsyncReadExt;
use tokio_stream::wrappers::BroadcastStream;

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

async fn serve(
    req: Request<Body>,
    stream: BroadcastStream<Bytes>,
) -> Result<Response<Body>, BoxedError> {
    eprintln!("uri: {}", req.uri());

    // load tinta
    // let mut tinta = tokio::fs::File::open("examples/tinta.jpg").await?;
    // let mut buf = vec![];
    // tinta.read_to_end(&mut buf).await?;
    // let buflen = buf.len();
    // let buf = Bytes::from(buf);

    let content_type: HeaderValue = "image/jpeg".parse()?;
    // let content_length: HeaderValue = format!("{buflen}").parse()?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, content_type);
    // headers.insert(CONTENT_LENGTH, content_length);

    let stream = stream
        .map_ok(move |body| {
            let len = body.len();
            let mut headers = headers.clone();
            let content_length: HeaderValue = format!("{len}").parse().unwrap();
            headers.insert(CONTENT_LENGTH, content_length);
            let part = multipart_stream::Part {
                headers,
                body,
            };
            // let res: Result<_, Infallible> = Ok(part);
            // res                   
            part
        });

    // let stream = futures::stream::repeat_with(move || {
    //     let part = multipart_stream::Part {
    //         headers: headers.clone(),
    //         body: buf.clone(),
    //     };
    //     let res: Result<_, Infallible> = Ok(part);
    //     res
    // });

    // let stream = futures::stream::unfold(0, |state| async move {

    //     let body = Bytes::from(format!("part {}", state));
    //     let part = multipart_stream::Part {
    //         headers: HeaderMap::new(),
    //         body,
    //     };
    //     match state {
    //         10 => None,
    //         _ => Some((Ok::<_, std::convert::Infallible>(part), state + 1)),
    //     }
    // });
    let stream = multipart_stream::serialize(stream, "foo");

    Ok(hyper::Response::builder()
        .header(
            http::header::CONTENT_TYPE,
            "multipart/x-mixed-replace; boundary=foo",
        )
        .body(hyper::Body::wrap_stream(stream))?)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BoxedError> {
    // let stream = warp::path!("stream" / String)
    //     .then(stream);
    // warp::serve(stream)
    //     .run(([127, 0, 0, 1], 3000))
    //     .await;

    // load tinta
    let mut tinta = tokio::fs::File::open("examples/tinta_helada.jpg").await?;
    let mut buf = vec![];
    tinta.read_to_end(&mut buf).await?;
    let buf = Bytes::from(buf);

    let (tx, _) = tokio::sync::broadcast::channel(16);

    let tx2 = tx.clone();

    tokio::spawn(async move {
        loop {
            let buf = buf.clone();
            tokio::time::sleep(Duration::from_secs(1)).await;
            match tx.send(buf) {
                Ok(subscriptions) => eprintln!("Sent image to {subscriptions} connections"),
                Err(_) => eprintln!("Nobody connected"),
            }

        }
    });

    let addr = ([127, 0, 0, 1], 3000).into();
    let make_svc = make_service_fn(move |_conn| {
        let tx = tx2.clone();
        futures::future::ok::<_, std::convert::Infallible>(service_fn(move |req| {
            let rx = tx.subscribe();
            let stream = BroadcastStream::new(rx);
            serve(req, stream)
        }))
    });
    let server = hyper::Server::bind(&addr).serve(make_svc);
    println!("Serving on http://{}", server.local_addr());
    server.await.unwrap();
    Ok(())
}
