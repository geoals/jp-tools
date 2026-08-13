//! Print the CompactDef system prompt exactly as it is sent.
//!
//!     cargo run -p jp-mine-core --example print_prompt

fn main() {
    println!("{}", jp_mine_core::compactdef::system_prompt());
}
