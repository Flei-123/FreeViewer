use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, error};

use freeviewer::host::FreeViewerHost;
use freeviewer::security::SecurityManager;

/// FreeViewer daemon for unattended access
/// This runs as a system service and allows incoming connections
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("Starting FreeViewer daemon v{}", env!("CARGO_PKG_VERSION"));

    // Generate a unique partner ID for this daemon
    let partner_id = generate_daemon_id();
    info!("Daemon Partner ID: {}", partner_id);

    // Initialize security
    let security_manager = Arc::new(Mutex::new(SecurityManager::new()));

    // Create and start the host
    let host = FreeViewerHost::new(partner_id.clone());
    
    match host.start().await {
        Ok(_) => {
            info!("FreeViewer daemon started successfully");
            info!("Partner ID: {}", partner_id);
            info!("Listening for incoming connections...");
            
            // Keep the daemon running
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                
                if !host.is_running().await {
                    error!("Host service stopped unexpectedly");
                    break;
                }
            }
        }
        Err(e) => {
            error!("Failed to start FreeViewer daemon: {}", e);
            return Err(e.into());
        }
    }

    info!("FreeViewer daemon shutting down");
    Ok(())
}

fn generate_daemon_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    
    // Generate a 9-digit ID like TeamViewer, but with a special prefix for daemons
    format!("D{:02} {:03} {:03}", 
        rng.gen_range(10..=99),
        rng.gen_range(100..=999),
        rng.gen_range(100..=999)
    )
}
