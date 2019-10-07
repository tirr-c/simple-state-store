use simple_state_store::{State, Store};

#[derive(Debug, Default)]
struct TestState(usize);

impl TestState {
    fn inner(&self) -> usize {
        self.0
    }
}

#[derive(Debug)]
enum Action {
    Foo(usize),
    Reset,
}

impl State for TestState {
    type Action = Action;
    fn mutate(&mut self, action: Action) {
        match action {
            Action::Foo(0) => {},
            Action::Foo(_) => self.0 += 1,
            Action::Reset => self.0 = 0,
        }
    }
}

fn main() {
    let mut store = Store::<TestState>::with_default();
    store.add_effect(|action| {
        match action {
            Action::Foo(cnt) if *cnt > 0 => {
                let action = Box::pin(async_std::stream::once(Action::Foo(cnt - 1)));
                Some(action)
            },
            _ => None,
        }
    });
    async_std::task::block_on(async move {
        assert_eq!(store.state().await.inner(), 0);
        store.dispatch(Action::Foo(5)).await;
        assert_eq!(store.state().await.inner(), 5);
        store.dispatch(Action::Foo(3)).await;
        assert_eq!(store.state().await.inner(), 8);
        store.dispatch(Action::Reset).await;
        assert_eq!(store.state().await.inner(), 0);
    });
}
