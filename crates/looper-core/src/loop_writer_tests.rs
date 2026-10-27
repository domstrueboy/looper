use super::*;

use crate::loop_mirror::load_beats;
use crate::test_support::temp_dir;

#[test]
fn what_was_asked_is_on_disk_once_flushed() {
    let dir = temp_dir("writer-flush");
    let writer = LoopWriter::new(dir.clone());

    writer.save_beats(7);
    writer.flush();

    assert_eq!(load_beats(&dir), Some(7));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jobs_land_in_the_order_they_were_asked_for() {
    let dir = temp_dir("writer-order");
    let writer = LoopWriter::new(dir.clone());

    writer.save_beats(7);
    writer.delete();
    writer.flush();
    assert_eq!(load_beats(&dir), None, "a delete after a save wins");

    writer.delete();
    writer.save_beats(9);
    writer.flush();
    assert_eq!(load_beats(&dir), Some(9), "and a save after a delete");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dropping_it_finishes_what_was_queued() {
    let dir = temp_dir("writer-drop");
    let writer = LoopWriter::new(dir.clone());

    writer.save_beats(5);
    drop(writer);

    assert_eq!(load_beats(&dir), Some(5));
    let _ = std::fs::remove_dir_all(&dir);
}
