use std::pin::Pin;
use async_std::{
    stream::Stream,
    task::{Context, Poll},
};

pub struct VecMergeStream<S: Stream> {
    streams: Vec<Option<S>>,
    items: Vec<S::Item>,
}

impl<S: Stream> Default for VecMergeStream<S> {
    fn default() -> Self {
        Self {
            streams: Vec::new(),
            items: Vec::new(),
        }
    }
}

impl<S: Stream> std::fmt::Debug for VecMergeStream<S> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stream_count = self.streams.iter().filter_map(|stream| stream.as_ref().map(drop)).count();
        write!(
            fmt,
            "VecMergeStream ({} stream{})",
            stream_count,
            if stream_count == 1 { "" } else { "s" },
        )
    }
}

impl<S: Stream + Unpin> Unpin for VecMergeStream<S> {}

impl<S: Stream> VecMergeStream<S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, stream: S) {
        let stream = Some(stream);
        if let Some(slot) = self.streams.iter_mut().find(|stream| stream.is_none()) {
            *slot = stream;
        } else {
            self.streams.push(stream);
        }
    }
}

impl<S: Stream> Extend<S> for VecMergeStream<S> {
    fn extend<T: IntoIterator<Item = S>>(&mut self, iter: T) {
        let mut iter = iter.into_iter().map(Some);
        let slots = self.streams.iter_mut().filter_map(|stream| {
            if stream.is_some() {
                None
            } else {
                Some(stream)
            }
        });
        for slot in slots {
            if let Some(stream) = iter.next() {
                *slot = stream;
            } else {
                return;
            }
        }
        self.streams.extend(iter);
    }
}

impl<S: Stream + Unpin> Stream for VecMergeStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        if let Some(item) = self.items.pop() {
            return Poll::Ready(Some(item));
        }
        let (items, all_done) = self.streams
            .iter_mut()
            .rev()
            .filter_map(|stream_wrapper| {
                if let Some(stream) = stream_wrapper {
                    let poll_result = Pin::new(stream).poll_next(cx);
                    if let Poll::Ready(None) = &poll_result {
                        *stream_wrapper = None;
                    }
                    Some(poll_result)
                } else {
                    None
                }
            })
            .fold((Vec::new(), true), |(mut items, all_done), poll_result| {
                match poll_result {
                    Poll::Ready(None) => return (items, all_done),
                    Poll::Ready(Some(item)) => items.push(item),
                    Poll::Pending => {},
                }
                (items, false)
            });
        self.items = items;
        if all_done {
            Poll::Ready(None)
        } else if let Some(item) = self.items.pop() {
            Poll::Ready(Some(item))
        } else {
            Poll::Pending
        }
    }
}
