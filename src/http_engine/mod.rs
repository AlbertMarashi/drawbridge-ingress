mod bytes;
use httparse;
use tokio::{net::TcpStream};
use tokio::io::AsyncReadExt;

use crate::{IngressLoadBalancerError, Code};

const BUFFER_SIZE: usize = 1024;

// async fn handle_tcp_stream(mut tcp_stream: TcpStream) -> Result<(), IngressLoadBalancerError> {
//     let mut headers = Vec::new();
//     let mut req = httparse::Request::new(&mut headers);



//     let mut data = vec![0; BUFFER_SIZE];

//     let old_len = data.len();
//     let new_len = old_len + BUFFER_SIZE;

//     data.resize(new_len, 0u8);


//     let mut data = vec![0; BUFFER_SIZE];
//     loop {
//         let mut buf = [0; BUFFER_SIZE];
//         let n = tcp_stream.read(&mut data).await
//             .map_err(|_| IngressLoadBalancerError::general(Code::HttpError, "Could not "))?;
//         if n == 0 {
//             break;
//         }

//         data.extend_from_slice(&buf);

//         let result = req.parse(&data)
//             .map_err(|_| IngressLoadBalancerError::general(Code::HttpError, "Could not parse http request"))?;

//         if let httparse::Status::Complete(len) = result {
//             break;
//         }
//     }

//     Ok(())
// }


// #[cfg(test)]
// mod tests {
//     use tokio::{net::TcpListener, io::AsyncWriteExt};

//     use super::*;

//     #[tokio::test]
//     async fn test_handle_tcp_stream() {
//         let server = TcpListener::bind("0.0.0.0:4000").await.unwrap();

//         let handle = tokio::spawn(async move {
//             while let Ok((stream, _)) = server.accept().await {
//                 handle_tcp_stream(stream).await.unwrap();
//             }
//         });

//         let mut client = TcpStream::connect("127.0.0.1:4000").await.unwrap();

//         let data = b"GET / HTTP/1.1\r\nHost: localhost:4000\r\n\r\n";

//         client.write_all(data).await.unwrap();

//         // wait for the server to finish
//         handle.await.unwrap();
//     }
// }

