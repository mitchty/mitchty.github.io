use crate::{
    data::{DataItem, MnistBatcher, NpzDataset, load_kaggle_item},
    training::TrainingConfig,
};
use burn::data::dataset::Dataset;
use burn::{
    data::{dataloader::batcher::Batcher, dataset::vision::MnistItem as BurnMnistItem},
    prelude::*,
    record::{CompactRecorder, Recorder},
};

fn run_infer<B: Backend>(artifact_dir: &str, device: &B::Device, item: DataItem) {
    let config = TrainingConfig::load(format!("{artifact_dir}/config.json"))
        .expect("Config should exist for the model; run train first");
    let record = CompactRecorder::new()
        .load(format!("{artifact_dir}/model").into(), device)
        .expect("trained model should exist; run train first");

    let model = config.model.init::<B>(device).load_record(record);

    let label = item.label;
    let batcher = MnistBatcher::default();
    let batch = batcher.batch(vec![item], device);
    let output = model.forward(batch.images);
    let predicted = output.argmax(1).flatten::<1>(0, 1).into_scalar();

    tracing::info!(predicted = %predicted, label = %label, "inference result");
}

/// Infer a single item from burn's built-in mnist dataset
pub fn infer<B: Backend>(artifact_dir: &str, device: B::Device, item: BurnMnistItem) {
    run_infer::<B>(
        artifact_dir,
        &device,
        DataItem {
            image: item
                .image
                .iter()
                .flat_map(|row| row.iter().copied())
                .collect(),
            width: 28,
            height: 28,
            label: item.label as u32,
        },
    );
}

/// Infer using a row from a kaggle? style mnist csv file
pub fn infer_from_file<B: Backend>(
    artifact_dir: &str,
    device: B::Device,
    path: &str,
    index: usize,
) {
    let item = load_kaggle_item(path, index);
    run_infer::<B>(
        artifact_dir,
        &device,
        DataItem {
            image: item
                .image
                .iter()
                .flat_map(|row| row.iter().copied())
                .collect(),
            width: 28,
            height: 28,
            label: item.label as u32,
        },
    );
}

/// Infer using a single item from a paired npz dataset.
pub fn infer_from_npz<B: Backend>(
    artifact_dir: &str,
    device: B::Device,
    imgs_path: &str,
    labels_path: &str,
    index: usize,
) {
    let ds = NpzDataset::from_npz(imgs_path, labels_path);
    let item = ds.get(index).unwrap_or_else(|| {
        panic!(
            "index {index} out of range in provided npz dataset: len is {}",
            ds.len()
        )
    });
    run_infer::<B>(artifact_dir, &device, item);
}
