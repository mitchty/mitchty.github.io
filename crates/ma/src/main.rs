#![recursion_limit = "256"]
mod cli;
mod data;
mod etl;
mod inference;
mod kanjivg;
mod model;
mod training;

fn main() {
    cli::run();
}
