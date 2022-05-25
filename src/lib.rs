use std::future::Future;
use std::marker::Unpin;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::join;

#[cfg(feature = "runtime-async-std")]
use async_std::{
    channel::{bounded as channel, Receiver, Sender},
    io::{Error, Read as AsyncRead, ReadExt as _},
    task::spawn,
};

#[cfg(feature = "runtime-tokio")]
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, Error, ReadBuf},
    spawn,
    sync::mpsc::{channel, Receiver, Sender},
};

type BufReceiver = Receiver<Result<Vec<u8>, Error>>;
type BufSender = Sender<Result<Vec<u8>, Error>>;

type RecvState = Pin<
    Box<dyn Future<Output = (BufReceiver, Option<Result<Vec<u8>, Error>>)> + Send + Sync + 'static>,
>;

enum TeeReaderState {
    Idle(BufReceiver),
    Recv(RecvState),
}

pub struct TeeReader {
    state: Option<TeeReaderState>,
}

#[cfg(feature = "runtime-async-std")]
type PollReadRet = Poll<Result<usize, Error>>;
#[cfg(feature = "runtime-tokio")]
type PollReadRet = Poll<Result<(), Error>>;

impl<'a> AsyncRead for TeeReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        #[cfg(feature = "runtime-async-std")] buf: &mut [u8],
        #[cfg(feature = "runtime-tokio")] buf: &mut ReadBuf,
    ) -> PollReadRet {
        loop {
            match self.state.take().unwrap() {
                TeeReaderState::Recv(mut fut) => match fut.as_mut().poll(cx) {
                    Poll::Ready((receiver, Some(Ok(queued_buf)))) => {
                        self.state = Some(TeeReaderState::Idle(receiver));
                        #[cfg(feature = "runtime-async-std")]
                        return Poll::Ready(std::io::Read::read(&mut queued_buf.as_slice(), buf));
                        #[cfg(feature = "runtime-tokio")]
                        {
                            buf.put_slice(queued_buf.as_slice());
                            return Poll::Ready(Ok(()));
                        }
                    }
                    Poll::Ready((receiver, Some(Err(error)))) => {
                        self.state = Some(TeeReaderState::Idle(receiver));
                        return Poll::Ready(Err(error));
                    }
                    Poll::Ready((receiver, None)) => {
                        self.state = Some(TeeReaderState::Idle(receiver));
                        #[cfg(feature = "runtime-async-std")]
                        return Poll::Ready(Ok(0));
                        #[cfg(feature = "runtime-tokio")]
                        return Poll::Ready(Ok(()));
                    }
                    Poll::Pending => {
                        self.state = Some(TeeReaderState::Recv(fut));
                        return Poll::Pending;
                    }
                },
                #[allow(unused_mut)]
                TeeReaderState::Idle(mut receiver) => {
                    let fut = Box::pin(async move {
                        #[cfg(feature = "runtime-async-std")]
                        let buf = receiver.recv().await.ok();
                        #[cfg(feature = "runtime-tokio")]
                        let buf = receiver.recv().await;
                        (receiver, buf)
                    });
                    self.state = Some(TeeReaderState::Recv(fut));
                }
            }
        }
    }
}

async fn read_loop<R>(mut reader: R, sender1: BufSender, sender2: BufSender)
where
    R: AsyncRead + Send + Unpin + 'static,
{
    while !sender1.is_closed() && !sender2.is_closed() {
        let mut buf = [0; 8 * 1024];
        let res = reader.read(&mut buf).await;
        match res {
            Ok(read) => {
                let buf = buf[..read].to_vec();
                let fut1 = sender1.send(Ok(buf.clone()));
                let fut2 = sender2.send(Ok(buf));
                let _ = join!(fut1, fut2);
            }
            Err(error) => {
                let error1 = Error::from(error.kind());
                let error2 = Error::from(error.kind());
                let fut1 = sender1.send(Err(error1));
                let fut2 = sender2.send(Err(error2));
                let _ = join!(fut1, fut2);
            }
        }
    }
}

pub fn tee<R>(reader: R, cap: usize) -> (TeeReader, TeeReader)
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let (sender1, receiver1) = channel(cap);
    let (sender2, receiver2) = channel(cap);
    spawn(read_loop(reader, sender1, sender2));
    (
        TeeReader {
            state: Some(TeeReaderState::Idle(receiver1)),
        },
        TeeReader {
            state: Some(TeeReaderState::Idle(receiver2)),
        },
    )
}
