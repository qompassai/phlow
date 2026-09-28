// rust-cli-hello: trivial inert fixture CLI.
//
// Prints a greeting and exits. No arguments are read, no files are touched,
// no network is used. It exists so the example task manifest
// (evals/public/rust-cli-parse-001.toml) has a real fixture to point at.

fn main() {
    println!("hello from the inert fixture");
}
