use std::{
    collections::HashMap,
    mem,
    sync::{Arc, Weak},
    thread,
};

use bytes::Bytes;
use tokio::{
    runtime::Handle as RTHandle,
    sync::{
        broadcast::{self, Sender as BCSender},
        RwLock,
    },
};
use tokio_stream::wrappers::BroadcastStream;

pub mod simple_server;

/// Mjpeg crate error
#[derive(thiserror::Error, PartialEq, Debug)]
pub enum Error {
    /// Only one streamer instance can exist for a given stream
    #[error("Only one streamer instance can exist per streamid")]
    StreamerExists,
    /// Main Mjpeg instance was dropped
    #[error("Primary mjpeg handle was dropped")]
    MjpegDropped,
    /// Stream has not been not created
    #[error("Stream not instantiated")]
    StreamNotFound,
    /// Nobody is connected to stream
    #[error("Nobody connected to stream")]
    NobodyConnected,
}

mod bufsize {
    /// Internal tokio::broadcast::channel size
    pub struct StreamerBufSize(pub usize);

    impl From<usize> for StreamerBufSize {
        fn from(bufsize: usize) -> Self {
            Self(bufsize)
        }
    }

    impl Default for StreamerBufSize {
        fn default() -> Self {
            Self(16)
        }
    }
}

pub use bufsize::StreamerBufSize;

type SyncedMap = RwLock<HashMap<String, BCSender<Bytes>>>;

/// Streamer instance that allows user to push new jpeg frames
#[cfg_attr(test, derive(Debug))]
pub struct Streamer {
    streamid: String,
    sender: BCSender<Bytes>,
    parent: MjpegHandle,
}

impl Streamer {
    /// Push jpeg frame to stream
    /// 
    /// Returns:
    /// - Ok(connected_clients)
    pub fn send(&self, buf: Bytes) -> Result<usize, Error> {
        if !self.parent.is_valid() {
            return Err(Error::MjpegDropped);
        }
        if self.sender.receiver_count() == 0 {
            // early return with this error
            return Err(Error::NobodyConnected);
        }
        self.sender.send(buf).map_err(|_e| Error::NobodyConnected)
    }
}

impl Drop for Streamer {
    fn drop(&mut self) {
        let map_opt = self.parent.map.upgrade();
        if let Some(map) = map_opt {
            // String::default -> String::new -> Vec::new does not allocate
            let streamid = mem::take(&mut self.streamid);
            // AsyncDrop is not a thing yet
            let handle = RTHandle::current();
            let join_res = thread::spawn(move || {
                // let handle = RTHandle::current();
                // let _rtguard = handle.enter();
                handle.block_on(async { map.write().await.remove(&streamid) })
            })
            .join();
            if let Err(e) = join_res {
                log::trace!("Streamer panicked on removal: {e:?}");
            }
        }
    }
}

/// Mjpeg secondary handle
/// 
/// It can perform all operations that Mjpeg primary handle as long
/// as primary handle is still alive
/// 
/// Useful to control the lifetime of the application
#[derive(Clone)]
#[cfg_attr(test, derive(Debug))]
pub struct MjpegHandle {
    map: Weak<SyncedMap>,
}

impl MjpegHandle {
    fn is_valid(&self) -> bool {
        self.map.strong_count() > 0
    }

    pub async fn stream(
        &self,
        streamid: &str,
    ) -> Result<BroadcastStream<Bytes>, Error> {
        let map = self.map.upgrade().ok_or_else(|| Error::MjpegDropped)?;
        Mjpeg { map }.stream(streamid).await
    }

    pub async fn streamer(
        &self,
        streamid: &str,
        bufsize: impl Into<StreamerBufSize>,
    ) -> Result<Streamer, Error> {
        let map = self.map.upgrade().ok_or_else(|| Error::MjpegDropped)?;
        Mjpeg { map }.streamer(streamid, bufsize).await
    }
}

/// Mjpeg primary handle
/// 
/// Keeps track of all streams
#[derive(Clone, Default)]
pub struct Mjpeg {
    map: Arc<SyncedMap>,
}

impl Mjpeg {
    /// Return Mjpeg secondary handle
    pub fn handle(&self) -> MjpegHandle {
        MjpegHandle {
            map: Arc::downgrade(&self.map),
        }
    }

    /// Return stream with `streamid`
    /// 
    /// This will only work if a streamer has been created using
    /// Mjpeg::streamer
    pub async fn stream(
        &self,
        streamid: &str,
    ) -> Result<BroadcastStream<Bytes>, Error> {
        self.map
            .read()
            .await
            .get(streamid)
            .map(|sender| {
                let rx = sender.subscribe();
                BroadcastStream::new(rx)
            })
            .ok_or_else(|| Error::StreamNotFound)
    }

    /// Creates new streamer instance, effectively activating stream `streamid`
    /// 
    /// Only one Streamer instance can exist, so multiple calls with the same
    /// `streamid` will return error
    /// 
    /// `bufsize` translates to the buffer size of internal 
    /// `tokio::broadcast::channel`
    pub async fn streamer(
        &self,
        streamid: &str,
        bufsize: impl Into<StreamerBufSize>,
    ) -> Result<Streamer, Error> {
        let mut map = self.map.write().await;
        // there can only exist 1 streamer
        if map.contains_key(streamid) {
            return Err(Error::StreamerExists);
        }
        let StreamerBufSize(bufsize) = bufsize.into();
        let (sender, _) = broadcast::channel(bufsize);
        map.insert(streamid.to_string(), sender.clone());
        Ok(Streamer {
            streamid: streamid.to_string(),
            sender,
            parent: self.handle(),
        })
    }
}

#[cfg(test)]
mod test {
    use futures::StreamExt;

    use super::*;

    #[tokio::test]
    async fn get_stream() {
        let mjpeg = Mjpeg::default();
        let _streamer = mjpeg.streamer("asd", 42).await.unwrap();
        let _stream_some = mjpeg.stream("asd").await.unwrap();
        let stream_non = mjpeg.stream("asdf").await;
        assert!(stream_non.is_err())
    }

    #[tokio::test]
    async fn make_streamer() {
        let mjpeg = Mjpeg::default();
        let s = mjpeg
            .streamer("somestreamer", StreamerBufSize::default())
            .await;
        assert!(s.is_ok());
        let o = mjpeg.streamer("somestreamer", 42).await;
        assert_eq!(o.unwrap_err(), Error::StreamerExists)
    }

    #[tokio::test]
    async fn drop_test() {
        // instantiate mjpeg
        let mjpeg = Mjpeg::default();

        // create stream "uff"
        let streamer = mjpeg
            .streamer("uff", StreamerBufSize::default())
            .await
            .unwrap();

        // create listener for uff
        let mut stream = mjpeg.stream("uff").await.unwrap();

        // send dummy
        // somebody already connected -- should not error out with
        // NobodyConnected
        streamer.send(Bytes::from([0].as_ref())).unwrap();

        // receive dummy
        stream.next().await;

        // there should be 1 stream
        let streams = mjpeg.map.read().await.len();
        assert_eq!(streams, 1);

        // after dropping streamer
        // it should be removed from map
        drop(streamer);
        let streams = mjpeg.map.read().await.len();
        assert_eq!(streams, 0);

        // Now stream should be exhausted; since all senders (Streamer)
        // were dropped
        let finished = stream.next().await.is_none();
        assert!(finished)
    }

    #[tokio::test]
    async fn handle_invalidated() {
        // create mjpeg instance
        let mjpeg = Mjpeg::default();

        // create streamer asd
        let streamer = mjpeg
            .streamer("asd", StreamerBufSize::default())
            .await
            .unwrap();

        // create handles
        let handle1 = mjpeg.handle();
        let handle2 = handle1.clone();

        // try to create asd again with handle
        let err = handle1.streamer("asd", 2).await.unwrap_err();
        assert_eq!(err, Error::StreamerExists);

        // create streamer qwe
        let _streamer2 = handle1.streamer("qwe", 2).await.unwrap();

        // get stream asd
        let _stream1 = handle2.stream("asd").await.unwrap();

        // drop mjpeg
        drop(mjpeg);

        // try to send dummy
        let err = streamer.send(Bytes::from([0].as_ref())).unwrap_err();
        assert_eq!(err, Error::MjpegDropped);
    }
}
