// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
use swankit_cli::wg;
fn main() {
    if let Err(error) = wg::decode(std::env::args().skip(1).collect()) {
        eprintln!("wg-decode: {error}");
        std::process::exit(1);
    }
}
