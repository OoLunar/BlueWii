use std::{
    fmt::{Debug, Display},
    process::exit,
};

use log::error;

pub trait FormattedUnwrap<T> {
    fn unwrap_or_fmt(self) -> T;
}

impl<T, E: Display + Debug> FormattedUnwrap<T> for Result<T, E> {
    fn unwrap_or_fmt(self) -> T {
        if cfg!(debug_assertions) {
            self.unwrap()
        } else {
            self.unwrap_or_else(|e| {
                error!("{}", e);
                exit(1)
            })
        }
    }
}
