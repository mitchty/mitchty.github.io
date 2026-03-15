pub mod cnn;
pub mod dense;

#[allow(unused_imports)]
pub use cnn::{ConvVAE, ConvVaeConfig};
#[allow(unused_imports)]
pub use dense::{DenseVAE, DenseVaeConfig};
