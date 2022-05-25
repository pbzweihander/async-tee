use futures_util::try_join;
use tokio::io::AsyncReadExt;

use async_tee::tee;

#[tokio::test]
async fn test_tokio_file() {
    let original_reader = tokio::fs::File::open(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("testfile"),
    )
    .await
    .unwrap();
    let (mut tee_reader1, mut tee_reader2) = tee(original_reader, 1);

    let output_fut1 = tokio::spawn(async move {
        let mut output_buf = Vec::new();
        tee_reader1
            .read_to_end(&mut output_buf)
            .await
            .map(|_| output_buf)
    });
    let output_fut2 = tokio::spawn(async move {
        let mut output_buf = Vec::new();
        tee_reader2
            .read_to_end(&mut output_buf)
            .await
            .map(|_| output_buf)
    });

    let (output_buf1, output_buf2) = try_join!(output_fut1, output_fut2).unwrap();

    assert_eq!(&b"foobar"[..], &output_buf1.unwrap()[..]);
    assert_eq!(&b"foobar"[..], &output_buf2.unwrap()[..]);
}
