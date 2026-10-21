use std::cell::RefCell;

use slint::ComponentHandle;

type Follower = Box<dyn Fn(i32) -> bool>;

thread_local! {
    static FOLLOWERS: RefCell<Vec<Follower>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn follow<C: ComponentHandle + 'static>(component: &C, set: fn(&C, i32)) {
    set(component, crate::preferences::theme_mode());
    let weak = component.as_weak();
    FOLLOWERS.with_borrow_mut(|followers| {
        followers.retain(|follow| follow(crate::preferences::theme_mode()));
        followers.push(Box::new(move |mode| match weak.upgrade() {
            Some(component) => {
                set(&component, mode);
                true
            }
            None => false,
        }));
    });
}

pub(super) fn apply(mode: i32) {
    FOLLOWERS.with_borrow_mut(|followers| followers.retain(|follow| follow(mode)));
}
