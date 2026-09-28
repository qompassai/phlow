//! Adversarial tests: malformed inputs are rejected with typed errors, and
//! failures never publish partial state.

use phlow_compute::{
    ComputeError, ExclusiveOutput, GridGeometry, INPUT_COUNT_MAX, LaunchArgs, Partition,
    SharedInput, TensorShape, TileExecutor, TileShape,
};

fn tensor_1d(extent: u32) -> TensorShape {
    TensorShape::new(&[extent]).expect("valid tensor shape")
}

fn tile_1d(extent: u32) -> TileShape {
    TileShape::new(&[extent]).expect("valid tile shape")
}

fn valid_output_1d(handle: u64) -> ExclusiveOutput {
    let partition = Partition::new(tensor_1d(1024), tile_1d(128)).expect("valid partition");
    ExclusiveOutput::new(handle, partition)
}

#[test]
fn rank_zero_is_rejected() {
    assert!(matches!(
        TileShape::new(&[]),
        Err(ComputeError::InvalidRank { rank: 0 })
    ));
}

#[test]
fn rank_above_max_is_rejected() {
    assert!(matches!(
        TileShape::new(&[1, 1, 1, 1, 1]),
        Err(ComputeError::InvalidRank { rank: 5 })
    ));
}

#[test]
fn zero_extent_is_rejected() {
    assert!(matches!(
        TileShape::new(&[16, 0]),
        Err(ComputeError::ZeroExtent { axis: 1 })
    ));
}

#[test]
fn extent_above_max_is_rejected() {
    assert!(matches!(
        TileShape::new(&[1025]),
        Err(ComputeError::ExtentTooLarge {
            axis: 0,
            extent: 1025
        })
    ));
}

#[test]
fn element_count_above_max_is_rejected() {
    // 1024^4 = 2^40 elements: each axis is legal alone, the product is not.
    assert!(matches!(
        TileShape::new(&[1024, 1024, 1024, 1024]),
        Err(ComputeError::ElementCountTooLarge { .. })
    ));
}

#[test]
fn rank_mismatch_is_rejected() {
    let tensor = TensorShape::new(&[64, 64]).expect("valid tensor");
    let err = Partition::new(tensor, tile_1d(16)).expect_err("ranks differ");
    assert_eq!(
        err,
        ComputeError::RankMismatch {
            tensor_rank: 2,
            tile_rank: 1
        }
    );
}

#[test]
fn non_divisible_axis_is_rejected() {
    let err = Partition::new(tensor_1d(1000), tile_1d(128)).expect_err("1000 % 128 != 0");
    assert_eq!(
        err,
        ComputeError::NotDivisible {
            axis: 0,
            tensor_extent: 1000,
            tile_extent: 128
        }
    );
}

#[test]
fn too_many_inputs_are_rejected() {
    let inputs: Vec<SharedInput> = (1..=INPUT_COUNT_MAX as u64 + 1)
        .map(SharedInput::new)
        .collect();
    let err = LaunchArgs::new(valid_output_1d(99), &inputs).expect_err("9 inputs");
    assert_eq!(err, ComputeError::TooManyInputs { count: 9 });
}

#[test]
fn no_inputs_is_rejected() {
    assert_eq!(
        LaunchArgs::new(valid_output_1d(99), &[]),
        Err(ComputeError::NoInputs)
    );
}

#[test]
fn input_aliasing_output_is_rejected() {
    let err = LaunchArgs::new(valid_output_1d(7), &[SharedInput::new(7)]).expect_err("alias");
    assert_eq!(err, ComputeError::HandleAlias { handle: 7 });
}

#[test]
fn tile_fault_aborts_with_the_failing_index() {
    let args = {
        let partition = Partition::new(tensor_1d(1024), tile_1d(128)).expect("valid partition");
        let output = ExclusiveOutput::new(7, partition);
        LaunchArgs::new(output, &[SharedInput::new(1)]).expect("valid args")
    };
    let mut ran = 0u64;
    let err = TileExecutor::execute(&args, |tile| {
        ran += 1;
        if tile.index == 3 {
            return Err("simulated tile failure");
        }
        Ok(())
    })
    .expect_err("tile 3 fails");
    assert_eq!(
        err,
        ComputeError::TileFault {
            index: 3,
            reason: "simulated tile failure"
        }
    );
    // Tiles 0..=3 ran; tiles 4..=7 never started.
    assert_eq!(ran, 4);
}

#[test]
fn tile_index_out_of_range_is_rejected() {
    let partition = Partition::new(tensor_1d(1024), tile_1d(128)).expect("valid partition");
    assert_eq!(
        partition.tile_origin(8),
        Err(ComputeError::TileIndexOutOfRange {
            index: 8,
            tile_count: 8
        })
    );
    assert_eq!(
        GridGeometry::new(&[4]).expect("valid grid").program_id(4),
        Err(ComputeError::ProgramIndexOutOfRange {
            index: 4,
            programs: 4
        })
    );
}
