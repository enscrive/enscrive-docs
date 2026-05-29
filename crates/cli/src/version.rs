pub const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "+",
    env!("ENSCRIVE_GIT_SHA"),
    " (",
    env!("ENSCRIVE_BUILD_DATE"),
    ")"
);
