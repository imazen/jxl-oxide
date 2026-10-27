#[test]
fn jxlp_out_of_order_minor_version_1() {
    let ooo = include_bytes!("jxlp_ooo.jxl");
    let ordered = include_bytes!("jxlp_ordered.jxl");

    let image_ooo = jxl_oxide::JxlImage::builder()
        .read(std::io::Cursor::new(ooo))
        .unwrap();
    let image_ordered = jxl_oxide::JxlImage::builder()
        .read(std::io::Cursor::new(ordered))
        .unwrap();

    assert_eq!(
        image_ooo.num_loaded_keyframes(),
        image_ordered.num_loaded_keyframes()
    );
    for idx in 0..image_ooo.num_loaded_keyframes() {
        let frame_ooo = image_ooo.render_frame(idx).unwrap();
        let frame_ordered = image_ordered.render_frame(idx).unwrap();
        for (grid_ooo, grid_ordered) in frame_ooo
            .image_planar()
            .into_iter()
            .zip(frame_ordered.image_planar())
        {
            assert_eq!(grid_ooo.buf(), grid_ordered.buf());
        }
    }
}

#[test]
fn jxlp_out_of_order_minor_version_0_rejected() {
    let data = include_bytes!("jxlp_ooo_v0.jxl");
    assert!(
        jxl_oxide::JxlImage::builder()
            .read(std::io::Cursor::new(data))
            .is_err()
    );
}
