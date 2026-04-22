use burn::{
    nn::{
        BatchNorm, BatchNormConfig, Dropout, DropoutConfig, Linear, LinearConfig, Relu,
        conv::{Conv2d, Conv2dConfig},
        pool::{AdaptiveAvgPool2d, AdaptiveAvgPool2dConfig},
    },
    prelude::*,
};

#[derive(Module, Debug)]
pub struct Model<B: Backend> {
    conv1: Conv2d<B>,
    bn1: BatchNorm<B>,
    conv2: Conv2d<B>,
    bn2: BatchNorm<B>,
    conv3: Conv2d<B>,
    bn3: BatchNorm<B>,
    pool: AdaptiveAvgPool2d,
    dropout: Dropout,
    linear1: Linear<B>,
    linear2: Linear<B>,
    activation: Relu,
}

#[derive(Config, Debug)]
pub struct ModelConfig {
    num_classes: usize,
    hidden_size: usize,
    #[config(default = "0.5")]
    dropout: f64,
    /// Output channels after conv1. conv2 doubles this, conv3 doubles again.
    /// Default 32 gives 32->64->128 to help keep gpu compute units busier.
    #[config(default = "32")]
    pub conv_channels: usize,
    /// Number of input channels in the image tensor.
    /// 1 = grayscale, 3 = three-channel (original + Otsu + Sauvola).
    /// Default 3 matches the `--three-channel` convert output.
    #[config(default = "3")]
    pub channels: usize,
}

impl ModelConfig {
    /// Returns the initialized model.
    ///
    /// Architecture:
    ///   conv1 (channels -> C, 3x3, pad=same) -> BN -> ReLU
    ///   conv2 (C -> 2C,       3x3, pad=same) -> BN -> ReLU
    ///   conv3 (2C -> 4C,      3x3, pad=same) -> BN -> ReLU
    ///   AdaptiveAvgPool(4x4) -> [B, 4C, 4, 4]
    ///   Linear(4Cx16 -> hidden_size) -> Dropout -> ReLU
    ///   Linear(hidden_size -> num_classes)
    ///
    /// With channels=3, C=32: input B,3,H,W -> 32->64->128 -> pool -> 2048 -> hidden -> classes.
    pub fn init<B: Backend>(&self, device: &B::Device) -> Model<B> {
        let c = self.conv_channels;
        Model {
            conv1: Conv2dConfig::new([self.channels, c], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn1: BatchNormConfig::new(c).init(device),
            conv2: Conv2dConfig::new([c, c * 2], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn2: BatchNormConfig::new(c * 2).init(device),
            conv3: Conv2dConfig::new([c * 2, c * 4], [3, 3])
                .with_padding(burn::nn::PaddingConfig2d::Same)
                .init(device),
            bn3: BatchNormConfig::new(c * 4).init(device),
            pool: AdaptiveAvgPool2dConfig::new([4, 4]).init(),
            activation: Relu::new(),
            linear1: LinearConfig::new(c * 4 * 4 * 4, self.hidden_size).init(device),
            linear2: LinearConfig::new(self.hidden_size, self.num_classes).init(device),
            dropout: DropoutConfig::new(self.dropout).init(),
        }
    }
}

impl<B: Backend> Model<B> {
    /// # Shapes
    ///   - Images: `[batch_size, channels, height, width]`
    ///   - Output: `[batch_size, num_classes]`
    pub fn forward(&self, images: Tensor<B, 4>) -> Tensor<B, 2> {
        let [batch_size, _channels, _height, _width] = images.dims();

        // Already [B, C, H, W] - no reshape needed.
        let x = images;

        // Block 1
        let x = self.conv1.forward(x);
        let x = self.bn1.forward(x);
        let x = self.activation.forward(x);

        // Block 2
        let x = self.conv2.forward(x);
        let x = self.bn2.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        // Block 3
        let x = self.conv3.forward(x);
        let x = self.bn3.forward(x);
        let x = self.activation.forward(x);
        let x = self.dropout.forward(x);

        // Pool + flatten: [B, 4C, 4, 4] -> [B, 4C*16]
        let x = self.pool.forward(x);
        let features = x.dims()[1] * x.dims()[2] * x.dims()[3];
        let x = x.reshape([batch_size, features]);

        // Classifier head
        let x = self.linear1.forward(x);
        let x = self.dropout.forward(x);
        let x = self.activation.forward(x);

        self.linear2.forward(x)
    }
}
