#![recursion_limit = "256"]
mod cli;
mod data;
mod etl;
mod filter_config;
mod kana_merging;
mod kanjivg;
mod model;
mod training;

fn main() {
    cli::run();
}
