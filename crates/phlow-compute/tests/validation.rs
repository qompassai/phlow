//! Validation tests: well-formed inputs behave as the contracts promise.

use phlow_compute::{
    ExclusiveOutput, ExecutionReport, GridGeometry, INPUT_COUNT_MAX, LaunchArgs, Partition,
    ProgramId, SharedInput, TILE_RANK_MAX, TensorShape, TileExecutor, TileShape, TileView,
};

fn tensor_1d(extent: u32) -> TensorShape {
    TensorShape::new(&[extent]).expect("valid tensor shape")
}

fn tile_1d(extent: u32) -> TileShape {
    TileShape::new(&[extent]).expect("valid tile shape")
}

fn launch_args_1d(tensor_extent: u32, tile_extent: u32) -> LaunchArgs {
    let partition =
        Partition::new(tensor_1d(tensor_extent), tile_1d(tile_extent)).expect("valid partition");
    let output = ExclusiveOutput::new(7, partition);
    LaunchArgs::new(output, &[SharedInput::new(1)]).expect("valid launch args")
}

#[test]
fn tile_shape_accepts_1d_extent() {
    let shape = TileShape::new(&[128]).expect("128 is a valid tile extent");
    assert_eq!(shape.rank(), 1);
    assert_eq!(shape.element_count(), 128);
    assert_eq!(shape.extents(), &[128]);
}

#[test]
fn tile_shape_accepts_max_rank() {
    let shape = TileShape::new(&[2, 2, 2, 2]).expect("rank 4 is the maximum");
    assert_eq!(shape.rank(), TILE_RANK_MAX);
    assert_eq!(shape.element_count(), 16);
}

#[test]
fn tile_shape_element_count_multiplies_axes() {
    let shape = TileShape::new(&[2, 3, 4]).expect("valid shape");
    assert_eq!(shape.element_count(), 24);
    assert_eq!(shape.extent(0), Some(2));
    assert_eq!(shape.extent(2), Some(4));
    assert_eq!(shape.extent(3), None);
}

#[test]
fn tensor_shape_accepts_large_extent() {
    let shape = TensorShape::new(&[1 << 20]).expect("1M elements fits the tensor bound");
    assert_eq!(shape.element_count(), 1 << 20);
}

#[test]
fn partition_infers_grid_from_division() {
    // Mirrors the cutile-rs launch-grid example: 1024 elements in tiles of
    // 128 give a grid of 8 tile programs.
    let partition = Partition::new(tensor_1d(1024), tile_1d(128)).expect("divisible");
    assert_eq!(partition.grid(), &[8]);
    assert_eq!(partition.tile_count(), 8);
}

#[test]
fn tile_origin_first_and_last_tile() {
    let partition = Partition::new(tensor_1d(1024), tile_1d(128)).expect("divisible");
    assert_eq!(partition.tile_origin(0).expect("index 0"), [0, 0, 0, 0]);
    let last = partition.tile_origin(7).expect("index 7");
    assert_eq!(last[0], 896);
}

#[test]
fn tiles_are_pairwise_disjoint() {
    let tensor = TensorShape::new(&[64, 64]).expect("valid tensor");
    let tile = TileShape::new(&[16, 16]).expect("valid tile");
    let partition = Partition::new(tensor, tile).expect("divisible");
    let mut tiles: Vec<([u32; TILE_RANK_MAX], [u32; TILE_RANK_MAX])> = Vec::new();
    partition
        .for_each_tile(|_, origin| {
            tiles.push((origin, [16, 16, 0, 0]));
            Ok::<(), ()>(())
        })
        .expect("iteration");
    assert_eq!(tiles.len(), 16);
    // Pairwise: no two tiles overlap on both axes.
    for (i, (origin_a, _)) in tiles.iter().enumerate() {
        for (origin_b, _) in &tiles[i + 1..] {
            let overlaps_x = origin_a[0] < origin_b[0] + 16 && origin_b[0] < origin_a[0] + 16;
            let overlaps_y = origin_a[1] < origin_b[1] + 16 && origin_b[1] < origin_a[1] + 16;
            assert!(
                !(overlaps_x && overlaps_y),
                "tiles {i} overlap at {origin_a:?} and {origin_b:?}"
            );
        }
    }
}

#[test]
fn launch_args_accept_max_inputs() {
    let partition = Partition::new(tensor_1d(256), tile_1d(64)).expect("divisible");
    let output = ExclusiveOutput::new(100, partition);
    let inputs: Vec<SharedInput> = (1..=INPUT_COUNT_MAX as u64).map(SharedInput::new).collect();
    let args = LaunchArgs::new(output, &inputs).expect("max inputs are valid");
    assert_eq!(args.input_count(), INPUT_COUNT_MAX);
    assert_eq!(args.inputs().count(), INPUT_COUNT_MAX);
}

#[test]
fn shared_input_is_copy_like_a_borrowed_handle() {
    let input = SharedInput::new(42);
    let copy = input;
    assert_eq!(input.handle(), copy.handle());
    assert_eq!(input, copy);
}

#[test]
fn program_id_roundtrips_through_linear_index() {
    let grid = GridGeometry::new(&[8, 4, 2]).expect("valid grid");
    assert_eq!(grid.programs(), 64);
    assert_eq!(grid.num_programs(0), Some(8));
    assert_eq!(grid.num_programs(3), None);
    for index in 0..64u64 {
        let id: ProgramId = grid.program_id(index).expect("in range");
        let back = grid.linear_index(id.coords()).expect("coords in grid");
        assert_eq!(back, index, "roundtrip failed for index {index}");
    }
}

#[test]
fn executor_runs_tiles_in_index_order() {
    let args = launch_args_1d(1024, 128);
    let mut seen: Vec<u64> = Vec::new();
    let report: ExecutionReport = TileExecutor::execute(&args, |tile: TileView| {
        seen.push(tile.index);
        assert_eq!(tile.rank, 1);
        Ok(())
    })
    .expect("all tiles succeed");
    let expected: Vec<u64> = (0..8).collect();
    assert_eq!(seen, expected);
    assert_eq!(
        report,
        ExecutionReport {
            tiles_total: 8,
            tiles_completed: 8,
        }
    );
}

#[test]
fn executor_covers_every_partition_tile() {
    let args = launch_args_1d(512, 64);
    let mut count = 0u64;
    TileExecutor::execute(&args, |_| {
        count += 1;
        Ok(())
    })
    .expect("all tiles succeed");
    assert_eq!(count, 8);
}
