# MJPEG Crate

**DISCLAIMER:** Work in progress

This intends to be a Rust crate to host multiple mjpeg streams using HTTP `x-mixed-replace` content type

There is another crate called
[mjpeg-rs](https://github.com/t924417424/mjpeg_rs.git), but it does not allow
multiple streams. Also it seems to be unmantained.

## Run example

```console
cargo run --example tinta
```

This example uses `mjpeg::simple_server::run` exposing http://localhost:3000.
The url path represents a streamid. Only one stream is setup at
http://localhost:3000/tinta, to test further, try creating new streamers which
feed into the `Mjpeg` handle. Those will get picked up by
http://localhost:3000/streamid