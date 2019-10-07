use std::ops::Deref;
use std::pin::Pin;

use async_std::{
    future,
    stream::Stream,
    sync::{RwLock, RwLockReadGuard},
    task::Poll,
};

use crate::util::VecMergeStream;

type Effect<A> = dyn Fn(&A) -> Option<Pin<Box<dyn Stream<Item = A> + Send>>> + Send + Sync;

/// A state that can be mutated using actions.
pub trait State {
    /// The type of action that this state accepts.
    type Action;

    /// Mutate the state using the given action.
    fn mutate(&mut self, action: Self::Action);
}

/// A state store that can be shared between threads.
///
/// The store owns a state, and invokes registered effects when actions are dispatched.
pub struct Store<S: State> {
    state: RwLock<S>,
    effects: Vec<Box<Effect<S::Action>>>,
}

impl<S: State> std::fmt::Debug for Store<S> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let effect_count = self.effects.len();
        write!(
            fmt,
            "Store ({} registered effect{})",
            effect_count,
            if effect_count == 1 { "" } else { "s" },
        )
    }
}

impl<S: State> Store<S> {
    /// Create a new store with the given initial state.
    pub fn new(initial_state: S) -> Self {
        Self {
            state: RwLock::new(initial_state),
            effects: Default::default(),
        }
    }

    /// Create a new store using the default state.
    pub fn with_default() -> Self
    where
        S: Default,
    {
        Self {
            state: Default::default(),
            effects: Default::default(),
        }
    }

    /// Register an effect.
    ///
    /// Effects are functions that receives a reference to each action and returns an asynchronous
    /// stream of actions to be dispatched. `None` can be returned if the effect decided to skip
    /// the given action.
    pub fn add_effect<E>(&mut self, effect: E) -> &mut Self
    where
        E: Fn(&S::Action) -> Option<Pin<Box<dyn Stream<Item = S::Action> + Send>>> + Send + Sync + 'static,
    {
        let effect = Box::new(effect) as Box<Effect<S::Action>>;
        self.effects.push(effect);
        self
    }

    /// Asynchronously retrieve a read-only handle to the state.
    ///
    /// This will acquire a read lock to the state. Any updates to the store will be blocked until
    /// all handles are dropped.
    pub async fn state(&self) -> StateReadHandle<'_, S> {
        let state = self.state.read().await;
        StateReadHandle(state)
    }
}

impl<S> Store<S>
where
    S: State,
    S::Action: Send,
{
    fn get_effect_streams(&self, action: &S::Action) -> Vec<Pin<Box<dyn Stream<Item = S::Action> + Send + '_>>> {
        self.effects.iter().filter_map(|effect| effect(action)).collect()
    }

    /// Dispatch a single action, and wait for all consequent effects to finish.
    pub async fn dispatch(&self, action: S::Action) {
        let mut streams = VecMergeStream::new();
        streams.extend(self.get_effect_streams(&action));

        let mut state = self.state.write().await;
        state.mutate(action);
        drop(state);

        self.dispatch_stream(streams).await
    }

    /// Dispatch a stream of actions, and wait for all consequent effects to finish.
    pub async fn dispatch_stream<'a, St>(&'a self, action_stream: St)
    where
        St: Stream<Item = S::Action> + Send + 'a,
    {
        let mut streams = VecMergeStream::new();
        streams.push(Box::pin(action_stream) as Pin<Box<dyn Stream<Item = S::Action> + Send + 'a>>);
        let mut streams = Pin::new(&mut streams);
        loop {
            let action = future::poll_fn(|cx| streams.as_mut().poll_next(cx)).await;
            if let Some(action) = action {
                let mut state = self.state.write().await;
                streams.extend(self.get_effect_streams(&action));
                state.mutate(action);
                // Don't release the lock while items are immediately available
                loop {
                    match future::poll_fn(|cx| Poll::Ready(streams.as_mut().poll_next(cx))).await {
                        Poll::Pending => break,
                        Poll::Ready(Some(action)) => {
                            streams.extend(self.get_effect_streams(&action));
                            state.mutate(action);
                        },
                        Poll::Ready(None) => return,
                    }
                }
            } else {
                return;
            }
        }
    }
}

/// A read-only handle to the state.
///
/// This handle holds a read lock to the state. Any updates to the store will be blocked until all
/// handles are dropped.
pub struct StateReadHandle<'a, S>(RwLockReadGuard<'a, S>);

impl<S> Deref for StateReadHandle<'_, S> {
    type Target = S;

    fn deref(&self) -> &S {
        &*self.0
    }
}
