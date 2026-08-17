use solcore_fuzz::{Target, fuzz};

fn main() {
    // SAFETY: This is the first operation in `main`, before any threads or
    // foreign runtime code that could access the environment are started.
    unsafe { fuzz(Target::Parser) };
}
