use std::collections::HashMap;

use futures_util::TryStreamExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::select;
use std::sync::{Arc, Mutex};
use tokio_util::codec::{BytesCodec, FramedRead};
use tokio::io::{self};

use tokio::io::AsyncWriteExt;
use std::net::SocketAddr;

struct ClientHandler {
    id: u64,
    p2p_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,
    p2p_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    stream: TcpStream,
}
impl ClientHandler {

    pub fn new(id: u64, p2p_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,  p2p_rx: mpsc::UnboundedReceiver<Vec<u8>>, stream: TcpStream) -> Self {
        ClientHandler {
            id,
            p2p_tx,
            p2p_rx,
            stream,
        }
    }

    async fn start(mut self) {


         // Process incoming messages from the client
        let ( r,  mut w) = io::split(self.stream);

        let mut framed: FramedRead<io::ReadHalf<TcpStream>, BytesCodec> = FramedRead::new(r, BytesCodec::new());
        loop {
            select! {
               
                result = framed.try_next() => {
                    match result {
                        Ok(Some(frame)) => {
                            // Process the received bytes
                            let u64_value: u64 = self.id as u64;
                            //println!("Received bytes: {:?}", frame);
                            self.p2p_tx.send((u64_value as u64, frame.to_vec())).unwrap();
                        }
                        Ok(None) => {
                            // Stream ended, send other side to close connection.
                            println!("Closing forward connection {}", self.id);
                            self.p2p_tx.send((self.id as u64, Vec::new())).unwrap();
                            break;
                        }
                        Err(err) => {
                            // Stream ended, send other side to close connection.
                            self.p2p_tx.send((self.id as u64, Vec::new())).unwrap();
                            eprintln!("Error receiving bytes: {:?} client id {}", err, self.id);
                            break;
                        }                        
                    }
                }                

                // Receive messages from the p2p client
                Some(msg) = self.p2p_rx.recv() =>{
                    if msg.is_empty() {
                        println!("Closing forward connection {}", self.id);
                        break;
                    }
                    // Send the message to the client socket
                    if let Err(_) = w.write_all(msg.as_ref()).await{
                        break; // Error occurred while sending, terminate the loop
                    }
 
                }
      
            }    
        }

    }


}

pub struct PFForwarder {
    clients: Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>>,
    p2p_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,
    server_address: SocketAddr,
}
impl PFForwarder {
        pub fn new(p2p_tx:mpsc::UnboundedSender<(u64, Vec<u8>)> , server_address: SocketAddr) -> PFForwarder {
            let clients = Arc::new(Mutex::new(HashMap::new()));
    
            PFForwarder {
                clients,
                p2p_tx,
                server_address
            }
  

        }


    // P2P send data to the mentioned client by client_id
    pub async fn  send(&self, client_id: u64, message: Vec<u8>) -> Result<(),    String> {
        // Check if the client exists in the HashMap
        {
            if let Some(client) = self.clients.lock().unwrap().get(&client_id) {
                // Send the message to the client
                if let Err(_) = client.send(message) {
                    return Err("Failed to send message to client".to_string());
                }
                return Ok(())
            }
        }

        // Create a new connection if that client id doesn't exists
        let  stream = TcpStream::connect(self.server_address).
        await.
        map_err(|err| format!("Connection error: {}", err))?;
        let _ = stream.set_nodelay(true);
        let client_id = client_id;
        println!("Opening forward connection for client {} ", client_id);

        // Clone necessary data for the client handler
        let clients_clone = self.clients.clone();
        let p2p_tx_clone = self.p2p_tx.clone();
        let (tx, p2p_rx) = mpsc::unbounded_channel();

        tx.send(message).map_err(|_| "Failed to send message to client")?;

        clients_clone.lock().unwrap().insert(client_id, tx.clone());
                 
        tokio::spawn( async move{
            let  client_handler = ClientHandler::new(client_id, p2p_tx_clone, p2p_rx, stream);
 
            client_handler.start().await;
    
            // Remove the client from the HashMap after handling is done
            clients_clone.lock().unwrap().remove(&client_id);              
        });     
        Ok(())
    }
}
