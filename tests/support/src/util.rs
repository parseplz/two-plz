use bytes::Bytes;
use std::task::Poll;

pub fn byte_str(s: &str) -> two_plz::frame::BytesStr {
    two_plz::frame::BytesStr::try_from(Bytes::copy_from_slice(s.as_bytes()))
        .unwrap()
}

pub async fn yield_once() {
    let mut yielded = false;
    futures::future::poll_fn(move |cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().clone().wake();
            Poll::Pending
        }
    })
    .await;
}
