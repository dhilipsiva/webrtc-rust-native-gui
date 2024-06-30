use eframe::egui;
use log::info;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use webrtc::{
    api::{media_engine::MediaEngine, APIBuilder},
    ice_transport::ice_candidate::RTCIceCandidateInit,
    ice_transport::ice_gathering_state::RTCIceGatheringState,
    ice_transport::ice_server::RTCIceServer,
    peer_connection::{
        configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
        RTCPeerConnection,
    },
};

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
    peer_connection: Arc<tokio::sync::Mutex<Option<Arc<RTCPeerConnection>>>>,
    local_sdp: Arc<Mutex<String>>,
    remote_sdp: Arc<Mutex<String>>,
    ice_candidates: Arc<tokio::sync::Mutex<Vec<RTCIceCandidateInit>>>,
    tx: mpsc::Sender<String>,
    rx: Arc<tokio::sync::Mutex<mpsc::Receiver<String>>>,
}

impl WebRTCApp {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel(32);
        Self {
            peer_connection: Arc::new(tokio::sync::Mutex::new(None)),
            local_sdp: Arc::new(Mutex::new(String::new())),
            remote_sdp: Arc::new(Mutex::new(String::new())),
            ice_candidates: Arc::new(tokio::sync::Mutex::new(vec![])),
            tx,
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        }
    }
}

impl Clone for WebRTCApp {
    fn clone(&self) -> Self {
        Self {
            peer_connection: Arc::clone(&self.peer_connection),
            local_sdp: Arc::clone(&self.local_sdp),
            remote_sdp: Arc::clone(&self.remote_sdp),
            ice_candidates: Arc::clone(&self.ice_candidates),
            tx: self.tx.clone(),
            rx: Arc::clone(&self.rx),
        }
    }
}

impl WebRTCApp {
    async fn gather_ice_candidates(&self) {
        let pc = self.peer_connection.lock().await.clone();
        if let Some(pc) = pc {
            let mut gather_complete = false;
            while !gather_complete {
                let state = pc.ice_gathering_state();
                match state {
                    RTCIceGatheringState::Complete => {
                        gather_complete = true;
                    }
                    _ => tokio::time::sleep(tokio::time::Duration::from_millis(100)).await,
                }
            }
        }
    }
    async fn create_answer(&self) -> RTCSessionDescription {
        let pc = self.peer_connection.lock().await.clone();
        if let Some(pc) = pc {
            info!("Creating answer...");
            match pc.create_answer(None).await {
                Ok(answer) => {
                    pc.set_local_description(answer.clone()).await.unwrap();
                    if let Some(local_desc) = pc.local_description().await {
                        let mut local_sdp_clone = local_desc.sdp.clone();
                        info!("Answer created with SDP: {}", local_sdp_clone);
                        local_sdp_clone
                            .push_str("a=ice-ufrag:abcd\na=ice-pwd:efgh567890ijklmnopqrstu\n");
                        let mut local_sdp = self.local_sdp.lock().unwrap();
                        *local_sdp = local_sdp_clone.clone();
                        return local_desc.clone();
                    }
                }
                Err(err) => {
                    info!("Failed to create answer: {:?}", err);
                }
            }
        }
        panic!("Failed to create answer");
    }

    async fn set_local_sdp(&self, sdp: RTCSessionDescription) {
        let pc = self.peer_connection.lock().await.clone();
        if let Some(pc) = pc {
            pc.set_local_description(sdp).await.unwrap();
        }
    }

    async fn create_offer(&self) {
        let pc = self.peer_connection.lock().await.clone();
        if let Some(pc) = pc {
            info!("Creating offer...");
            let ice_candidates = Arc::clone(&self.ice_candidates);
            pc.on_ice_candidate(Box::new(move |candidate| {
                dbg!(&candidate);
                let ice_candidates = Arc::clone(&ice_candidates);
                Box::pin(async move {
                    if let Some(candidate) = candidate {
                        let mut ice_candidates = ice_candidates.lock().await;
                        ice_candidates.push(candidate.to_json().unwrap());
                    }
                })
            }));

            match pc.create_offer(None).await {
                Ok(offer) => {
                    pc.set_local_description(offer.clone()).await.unwrap();
                    dbg!("before");
                    self.gather_ice_candidates().await;

                    let mut promise = pc.gathering_complete_promise().await;
                    let _ = promise.recv();
                    dbg!("after");
                    // // Ensure local description has ICE candidates and credentials
                    // while pc.ice_gathering_state() != RTCIceGatheringState::Complete {
                    //     tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    // }

                    if let Some(local_desc) = pc.local_description().await {
                        let mut local_sdp_clone = local_desc.sdp.clone();
                        info!("Offer created with SDP: {}", local_sdp_clone);
                        local_sdp_clone
                            .push_str("a=ice-ufrag:abcd\na=ice-pwd:efgh567890ijklmnopqrstu\n");
                        let mut local_sdp = self.local_sdp.lock().unwrap();
                        *local_sdp = local_sdp_clone
                    }
                }
                Err(err) => {
                    info!("Failed to create offer: {:?}", err);
                }
            }
        } else {
            info!("Peer connection is not initialized");
        }
    }

    async fn handle_answer(&self) {
        let pc = self.peer_connection.lock().await.clone();
        if let Some(pc) = pc {
            let remote_sdp_clone = {
                let remote_sdp = self.remote_sdp.lock().unwrap();
                remote_sdp.clone()
            };
            let answer = RTCSessionDescription::answer(remote_sdp_clone.clone()).unwrap();
            match pc.set_remote_description(answer).await {
                Ok(ok) => {
                    info!("Remote description set: {:?}", ok);

                    // Add stored ICE candidates
                    let ice_candidates = self.ice_candidates.lock().await.clone();
                    for candidate in ice_candidates {
                        dbg!(&candidate);
                        pc.add_ice_candidate(candidate).await.unwrap();
                    }
                }
                Err(err) => {
                    info!("Failed to set remote description: {:?}", err);
                }
            }
        }
    }

    async fn create_peer_connection(&self) {
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs().unwrap();
        let api = APIBuilder::new().with_media_engine(media_engine).build();
        let config = RTCConfiguration {
            ice_servers: vec![
                RTCIceServer {
                    urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                    ..Default::default()
                },
                RTCIceServer {
                    urls: vec!["stun:stun1.l.google.com:19302".to_owned()],
                    ..Default::default()
                },
                RTCIceServer {
                    urls: vec!["stun:stun2.l.google.com:19302".to_owned()],
                    ..Default::default()
                },
            ],
            // ice_transport_policy: RTCIceTransportPolicy::Relay, // Use relay to enforce IPv4 use
            ..Default::default()
        };

        let peer_connection = api.new_peer_connection(config).await.unwrap();
        let mut pc = self.peer_connection.lock().await;
        *pc = Some(Arc::new(peer_connection));
    }
}

impl eframe::App for WebRTCApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let local_sdp = Arc::clone(&self.local_sdp);
        let remote_sdp = Arc::clone(&self.remote_sdp);

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

            if ui.button("Create Answer").clicked() {
                let app = self.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let answer = app.create_answer().await;
                    app.set_local_sdp(answer).await;
                    ctx.request_repaint();
                });
            }
        });
    }
}
