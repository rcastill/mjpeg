use std::env;
use std::io::Read;
use std::time::Duration;

use bytes::Bytes;
use mjpeg::{Mjpeg, StreamerBufSize};

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), BoxedError> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    env_logger::init();
    let mjpeg = Mjpeg::default();

    // load tinta
    let mut tinta = std::fs::File::open("examples/tinta_helada.jpg")?;
    let mut buf = vec![];
    tinta.read_to_end(&mut buf)?;
    let buf = Bytes::from(buf);

    // define stream "tinta"
    let tinta = mjpeg.streamer("tinta", StreamerBufSize::default()).await?;
    tokio::spawn(async move {
        loop {
            let buf = buf.clone();
            tokio::time::sleep(Duration::from_secs(1)).await;
            match tinta.send(buf) {
                Ok(subscriptions) => {
                    log::info!("Sent image to {subscriptions} connections")
                }
                Err(mjpeg::Error::MjpegDropped) => break,
                Err(e) => log::error!("{e}"),
            }
        }
    });

    // run server at localhost:3000
    mjpeg::simple_server::run(mjpeg.handle(), ([127, 0, 0, 1], 3000))
        .await
        .map_err(Into::into)
}
