use std::collections::HashMap;
use std::net::SocketAddr;
use futures_util::TryStreamExt;
use tokio::net::{TcpListener,TcpStream};
use tokio::sync::mpsc;
use tokio::select;
use std::sync::{Arc, Mutex};
use tokio_util::codec::{BytesCodec, FramedRead};
use tokio::io::{self};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncWriteExt;


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
                            println!("Closing listening connection {}", self.id);
                            self.p2p_tx.send((self.id as u64, Vec::new())).unwrap();
                            break;
                        }
                        Err(err) => {
                            // Handle the error, send other side to close connection.
                            self.p2p_tx.send((self.id as u64, Vec::new())).unwrap();
                            eprintln!("Error receiving bytes: {:?} client id {}", err, self.id);
                            break;
                        }                        
                    }
                }                

                // Receive messages from the p2p client
                Some(msg) = self.p2p_rx.recv() =>{
                    if msg.is_empty() {
                        println!("Closing listening connection {}", self.id);
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

pub struct PFListener {
    clients: Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>>,
    p2p_tx: mpsc::UnboundedSender<(u64, Vec<u8>)>,
    sock_counter:AtomicU64,
    listen_address: SocketAddr
}
impl PFListener {
    pub fn new(p2p_tx:mpsc::UnboundedSender<(u64, Vec<u8>)>,  listen_address: SocketAddr) -> PFListener {
        let clients = Arc::new(Mutex::new(HashMap::new()));
    
        PFListener {
            clients,
            p2p_tx,
            sock_counter: AtomicU64::new(1),
            listen_address: listen_address
        }
    }

    async fn run_server(&self ){
        // Start the server and accept client connections
        let listener = TcpListener::bind(self.listen_address ).await.unwrap();

        loop{
            let ( stream, _) = listener.accept().await.unwrap();
            let _ = stream.set_nodelay(true);
            let client_id = self.generate_client_id();

            // Clone necessary data for the client handler
            let clients_clone = self.clients.clone();
            let p2p_tx_clone = self.p2p_tx.clone();
            let (tx, p2p_rx) = mpsc::unbounded_channel();
            println!("Opening listening connection {} ", client_id);

            clients_clone.lock().unwrap().insert(client_id, tx);
                                
            tokio::spawn( async move{
                let  client_handler = ClientHandler::new(client_id, p2p_tx_clone, p2p_rx, stream);
 
                    client_handler.start().await;
    
                    // Remove the client from the HashMap after handling is done
                    clients_clone.lock().unwrap().remove(&client_id);              
            });
        }

    }

    fn generate_client_id( &self) -> u64 {
        // Generate a unique client ID based on the client's IP address and port
        return  self.sock_counter.fetch_add(1, Ordering::SeqCst);
    }

    pub fn start( arc_self: Arc<Self>) {
        tokio::spawn(async move  {
            arc_self.run_server().await;
        });
    }

    // P2P send data to the mentioned client by client_id
    pub fn send(&self, client_id: u64, message: Vec<u8>) -> Result<(), &'static str> {
        // Check if the client exists in the HashMap
        if let Some(client) = self.clients.lock().unwrap().get(&client_id) {
            // Send the message to the client
            client.send(message).map_err(|_| "Failed to send message to client")?;
            Ok(())
        } else {
            Err("Client not found")
        }
    }
}
