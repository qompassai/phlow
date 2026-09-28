//! `phlow` binary: thin entrypoint over the shared CLI implementation in
//! [`phlow_cli`].

#![forbid(unsafe_code)]

fn main() {
    std::process::exit(phlow_cli::run());
}
