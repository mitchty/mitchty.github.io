// VAE models not yet connected to inference kept for future possible use.
#![allow(dead_code)]

use crate::ai::reparameterize;
use burn::{
    config::Config,
    module::Module,
    nn::{
        Linear, LinearConfig, PaddingConfig2d, Relu,
        conv::{Conv2d, Conv2dConfig, ConvTranspose2d, ConvTranspose2dConfig},
    },
    tensor::{Tensor, backend::Backend},
};

/// Configuration for the Convolutional Variational Autoencoder
#[derive(Config, Debug)]
pub struct ConvVaeConfig {
    /// The size of the flattened input vector e.g., 28x28 = 784 for mnist datasets.
    /// Used primarily for compatibility with loss functions expecting flat vectors.
    #[config(default = 784)]
    pub input_dim: usize,
    /// Dimensionality of the latent space (z).
    #[config(default = 20)]
    pub latent_dim: usize,
    /// Learning rate for the optimizer.
    #[config(default = 1e-3)]
    pub learning_rate: f64,
    /// Total number of training epochs.
    #[config(default = 25)]
    pub num_epochs: usize,
    /// Mini-batch size for training and inference.
    #[config(default = 128)]
    pub batch_size: usize,
    /// The number of channels in the first convolutional layer.
    /// Subsequent layers typically double this depth (e.g., 32 -> 64).
    #[config(default = 32)]
    pub base_channels: usize,
}

/// The Convolutional Encoder Network.
///
/// Compresses input images into a low-dimensional latent space using
/// a series of downsampling convolutional layers.
#[derive(Module, Debug)]
pub struct ConvEncoder<B: Backend> {
    conv1: Conv2d<B>,
    conv2: Conv2d<B>,
    fc_mu: Linear<B>,
    fc_logvar: Linear<B>,
    activation: Relu,
    /// The size of the flattened feature map after the final convolution.
    /// Used to initialize the linear layers.
    flattened_dim: usize,
}

impl<B: Backend> ConvEncoder<B> {
    /// Constructs a new `ConvEncoder`.
    ///
    /// Initializes weights and computes feature map dimensions.
    pub fn new(config: &ConvVaeConfig, device: &B::Device) -> Self {
        let c = config.base_channels;

        let conv1 = Conv2dConfig::new([1, c], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1)) // Keeps alignment
            .init(device);

        let conv2 = Conv2dConfig::new([c, c * 2], [3, 3])
            .with_stride([2, 2])
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .init(device);

        let flattened_dim = (c * 2) * 7 * 7;

        let fc_mu = LinearConfig::new(flattened_dim, config.latent_dim).init(device);
        let fc_logvar = LinearConfig::new(flattened_dim, config.latent_dim).init(device);

        Self {
            conv1,
            conv2,
            fc_mu,
            fc_logvar,
            activation: Relu::new(),
            flattened_dim,
        }
    }

    pub fn forward(&self, x: Tensor<B, 4>) -> (Tensor<B, 2>, Tensor<B, 2>) {
        // Convolutions + ReLU
        let x = self.activation.forward(self.conv1.forward(x));
        let x = self.activation.forward(self.conv2.forward(x));

        // Flatten: [Batch, Channels, Height, Width] -> [Batch, Flattened_Dim]
        let x = x.flatten(1, 3);

        let mu = self.fc_mu.forward(x.clone());
        let sigma = self.fc_logvar.forward(x);

        (mu, sigma)
    }
}

/// The Convolutional Decoder Network.
///
/// Reconstructs images from the latent vectors using Transposed Convolutions via Upsampling.
#[derive(Module, Debug)]
pub struct ConvDecoder<B: Backend> {
    fc_initial: Linear<B>,
    convt1: ConvTranspose2d<B>,
    convt2: ConvTranspose2d<B>,
    activation: Relu,
    /// Base channel depth used to reshape the initial linear output.
    base_channels: usize,
}

impl<B: Backend> ConvDecoder<B> {
    pub fn new(config: &ConvVaeConfig, device: &B::Device) -> Self {
        let c = config.base_channels;

        // Must match encoder output dimensions: (c * 2) * 7 * 7
        let flattened_dim = (c * 2) * 7 * 7;

        let fc_initial = LinearConfig::new(config.latent_dim, flattened_dim).init(device);

        let convt1 = ConvTranspose2dConfig::new([c * 2, c], [3, 3])
            .with_stride([2, 2])
            .with_padding([1, 1])
            .with_padding_out([1, 1]) // Crucial for correct output size
            .init(device);

        let convt2 = ConvTranspose2dConfig::new([c, 1], [3, 3])
            .with_stride([2, 2])
            .with_padding([1, 1])
            .with_padding_out([1, 1])
            .init(device);

        Self {
            fc_initial,
            convt1,
            convt2,
            activation: Relu::new(),
            base_channels: c,
        }
    }

    pub fn forward(&self, z: Tensor<B, 2>) -> Tensor<B, 4> {
        // Expand latent vector
        let x = self.activation.forward(self.fc_initial.forward(z));

        // Unflatten/Reshape to [Batch, Channels, Height, Width]
        // Note: Reshape dimension must match the calculated flattened size.
        let x = x.reshape([0, (self.base_channels as i32 * 2), 7, 7]);

        // Upsampling layers
        let x = self.activation.forward(self.convt1.forward(x));

        // Final layer: No ReLU here, just Sigmoid for pixel range [0, 1]
        burn::tensor::activation::sigmoid(self.convt2.forward(x))
    }
}

/// The complete Convolutional Variational Autoencoder.
///
/// Wraps the `ConvEncoder` and `ConvDecoder` and implements reparameterization.
#[derive(Module, Debug)]
pub struct ConvVAE<B: Backend> {
    pub encoder: ConvEncoder<B>,
    pub decoder: ConvDecoder<B>,
}

impl<B: Backend> ConvVAE<B> {
    pub fn new(config: &ConvVaeConfig, device: &B::Device) -> Self {
        Self {
            encoder: ConvEncoder::new(config, device),
            decoder: ConvDecoder::new(config, device),
        }
    }

    pub fn forward(&self, x: Tensor<B, 4>) -> (Tensor<B, 2>, Tensor<B, 2>, Tensor<B, 2>) {
        // Encode first
        let (mu, sigma) = self.encoder.forward(x);

        // Reparameterize: z = mu + sigma * epsilon
        let z = reparameterize(mu.clone(), sigma.clone());

        // Decode third
        let recon_img = self.decoder.forward(z);

        // Flatten output for loss calculation: [Batch, 1, 28, 28] -> [Batch, 784]
        let recon_flat = recon_img.flatten(1, 3);

        (recon_flat, mu, sigma)
    }
}
