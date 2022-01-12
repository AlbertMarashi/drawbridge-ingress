use std::io::Read;

use tokio::io::{AsyncReadExt, Result as TokioResult, BufReader, AsyncRead};


struct Cursor<R> {
    inner: InnerIter<R>,
    pos: usize,
}

struct InnerIter<R> {
    iter: R,

    // Number of bytes we've read
    peeked_bytes: u32,
    peeked_byte: Option<u8>,
}

impl<R> InnerIter<R>
    where R: AsyncRead + Unpin
{
    pub async fn peek_byte(&mut self) -> TokioResult<Option<u8>> {
        // if there is already a byte in the peeked_byte cache, return it
        if let Some(byte) = self.peeked_byte {
            return Ok(Some(byte));
        }

        // otherwise, read a byte from the underlying reader
        let mut buf = [0; 1];
        let n = self.iter.read(&mut buf).await?;

        unimplemented!()
    }
}


// #[tokio::test]
// async fn test_bytes() {
//     let text = b"Hello, world!".to_vec();
//     let inner = BufReader::new(text);
//     let mut bytes = Bytes { inner: "hello".as_bytes() };
//     assert_eq!(bytes.next().await.unwrap().unwrap(), b'h');
//     assert_eq!(bytes.next().await.unwrap().unwrap(), b'e');
//     assert_eq!(bytes.next().await.unwrap().unwrap(), b'l');
//     assert_eq!(bytes.next().await.unwrap().unwrap(), b'l');
//     assert_eq!(bytes.next().await.unwrap().unwrap(), b'o');
// }