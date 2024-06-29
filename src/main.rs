use eframe::egui;
use log::info;
use std::sync::{Arc, Mutex};
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

#[tokio::main]
async fn main() {
    env_logger::init();
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "WebRTC Client",
        options,
        Box::new(|_cc| Box::new(WebRTCApp::new())),
    )
    .unwrap();
}

struct WebRTCApp {
    peer_connection: Arc<Mutex<Option<Arc<RTCPeerConnection>>>>,
    local_sdp: Arc<Mutex<String>>,
    remote_sdp: Arc<Mutex<String>>,
}

impl WebRTCApp {
    fn new() -> Self {
        Self {
            peer_connection: Arc::new(Mutex::new(None)),
            local_sdp: Arc::new(Mutex::new(String::new())),
            remote_sdp: Arc::new(Mutex::new(String::new())),
        }
    }

    async fn create_peer_connection(&self) {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs().unwrap();
        let api = APIBuilder::new().with_media_engine(media_engine).build();
        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let peer_connection = api.new_peer_connection(config).await.unwrap();
        let mut pc = self.peer_connection.lock().unwrap();
        *pc = Some(Arc::new(peer_connection));
    }

    async fn create_offer(&self) {
        let pc = self.peer_connection.lock().unwrap().clone();
        if let Some(pc) = pc {
            info!("Creating offer...");
            match pc.create_offer(None).await {
                Ok(offer) => match pc.set_local_description(offer.clone()).await {
                    Ok(_) => {
                        let mut local_sdp = self.local_sdp.lock().unwrap();
                        *local_sdp = offer.sdp;
                        info!("Offer created: {}", *local_sdp);
                    }
                    Err(err) => {
                        info!("Failed to set local description: {:?}", err);
                    }
                },
                Err(err) => {
                    info!("Failed to create offer: {:?}", err);
                }
            }
        } else {
            info!("Peer connection is not initialized");
        }
    }

    async fn handle_answer(&self) {
        let pc = self.peer_connection.lock().unwrap().clone();
        if let Some(pc) = pc {
            let remote_sdp = self.remote_sdp.lock().unwrap().clone();
            let answer = RTCSessionDescription::answer(remote_sdp).unwrap();
            match pc.set_remote_description(answer).await {
                Ok(_) => {
                    info!("Remote description set");
                }
                Err(err) => {
                    info!("Failed to set remote description: {:?}", err);
                }
            }
        }
    }
}

impl eframe::App for WebRTCApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let local_sdp = Arc::clone(&self.local_sdp);
        let remote_sdp = Arc::clone(&self.remote_sdp);
        let peer_connection = Arc::clone(&self.peer_connection);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("WebRTC Client");

            if ui.button("Initialize").clicked() {
                let app = self.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    app.create_peer_connection().await;
                    ctx.request_repaint();
                });
            }

            if ui.button("Create Offer").clicked() {
                let app = self.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    app.create_offer().await;
                    ctx.request_repaint();
                });
            }

            ui.horizontal(|ui| {
                ui.label("Local SDP:");
                let mut local_sdp = local_sdp.lock().unwrap();
                ui.text_edit_multiline(&mut *local_sdp);
            });

            ui.horizontal(|ui| {
                ui.label("Remote SDP:");
                let mut remote_sdp = remote_sdp.lock().unwrap();
                ui.text_edit_multiline(&mut *remote_sdp);
                if ui.button("Set Remote SDP").clicked() {
                    let app = self.clone();
                    let ctx = ctx.clone();
                    tokio::spawn(async move {
                        app.handle_answer().await;
                        ctx.request_repaint();
                    });
                }
            });
        });
    }
}

impl Clone for WebRTCApp {
    fn clone(&self) -> Self {
        Self {
            peer_connection: Arc::clone(&self.peer_connection),
            local_sdp: Arc::clone(&self.local_sdp),
            remote_sdp: Arc::clone(&self.remote_sdp),
        }
    }
}
