//! This module is responsible for processing visual information from the camera and detecting objects using a neural network model.

// Import the necessary standard library modules
use std::{
    sync::{
        mpsc::{Receiver, Sender}, // For sending and receiving messages between threads
        Arc,
        Mutex, // For sharing and synchronizing data between threads
    },
    thread::{self, JoinHandle}, // For creating and managing threads
    time::Duration,             // For representing time intervals
};

// Import the Detection type from the detector submodule
use self::detector::Detection;
// Import the RoktrackProperty type from the init submodule in the util module
use super::util::init::RoktrackProperty;

pub mod camera; // Declare the camera submodule
pub mod detector; // Declare the detector submodule

/// This enum defines the commands that can be used to control the vision thread.
pub enum VisionMgmtCommand {
    On,                    // Turn on the vision thread
    Off,                   // Turn off the vision thread
    SwitchSessionPylon,    // Switch to the pylon detection session
    SwitchSessionPylonOcr, // Switch to the pylon OCR detection session
    SwitchSessionAnimal,   // Switch to the animal detection session
    SwitchSz320,           // Switch to the 320x240 resolution
    SwitchSz640,           // Switch to the 640x480 resolution
}

/// This struct provides a means of image processing using a camera and a detector.
pub struct RoktrackVision {
    inner: Arc<Mutex<RoktrackVisionInner>>, // A shared and synchronized wrapper for the inner struct that contains the camera and detector fields
    property: Arc<RoktrackProperty>, // A shared wrapper for the property struct that contains the paths and configurations
    state: Arc<Mutex<bool>>,
}

/// This impl block defines the methods for the RoktrackVision struct.
impl RoktrackVision {
    /// This method creates a new instance of the RoktrackVision struct with the given property.
    pub fn new(property: RoktrackProperty) -> Self {
        Self {
            // Create a new Arc<Mutex<RoktrackVisionInner>> by calling the new method on the RoktrackVisionInner struct and cloning the property
            inner: Arc::new(Mutex::new(RoktrackVisionInner::new(property.clone()))),
            // Create a new Arc<RoktrackProperty> by calling the new method on the Arc type and passing the property
            property: Arc::new(property),
            state: Arc::new(Mutex::new(true)),
        }
    }

    /// This method spawns a new thread that runs the inference loop for image processing.
    /// It takes two arguments: a sender and a receiver for communicating with other threads.
    /// It returns a handle to the spawned thread.
    ///
    /// # Note: THIS THREAD IS SLOW LOOP.
    pub fn run(
        &self,
        tx: Sender<VisualInfo>, // The sender for sending the detection results as visual information to other threads
        rx: Receiver<VisionMgmtCommand>, // The receiver for receiving management commands from other threads
    ) -> JoinHandle<()> {
        let local_self = self.inner.clone(); // Clone the inner field to avoid borrowing issues
        let local_property = self.property.clone(); // Clone the property field to avoid borrowing issues
        let local_state = self.state.clone();

        // Spawn a new thread and run an infinite loop
        thread::spawn(move || loop {
            // Wait for a short time before repeating the loop
            thread::sleep(Duration::from_millis(10));

            // Read the management commands from the receiver and match them
            match rx.try_recv() {
                Ok(VisionMgmtCommand::Off) => {
                    *local_state.lock().unwrap() = false;
                    continue; // If the command is Off, skip the rest of the loop and try again
                }
                Ok(VisionMgmtCommand::On) => {
                    *local_state.lock().unwrap() = true;
                } // If the command is On, do nothing and proceed
                Ok(VisionMgmtCommand::SwitchSessionPylon) => {
                    log::debug!("Vision VisionMgmtCommand::SwitchSessionPylon Received");
                    let _ = local_self.lock().unwrap().det.build_pylon_sessions();
                }
                Ok(VisionMgmtCommand::SwitchSessionPylonOcr) => {
                    log::debug!("Vision VisionMgmtCommand::SwitchSessionPylonOcr Received");
                    // If the command is SwitchSessionPylonOcr, lock the inner field and update the detector sessions with the pylon OCR sessions
                    let _ = local_self.lock().unwrap().det.build_pylon_ocr_sessions();
                }
                Ok(VisionMgmtCommand::SwitchSessionAnimal) => {
                    log::debug!("Vision VisionMgmtCommand::SwitchSessionAnimal Received");
                    // If the command is SwitchSessionAnimal, lock the inner field and update the detector sessions with the animal sessions
                    let _ = local_self.lock().unwrap().det.build_animal_sessions();
                }
                Ok(VisionMgmtCommand::SwitchSz320) => {
                    log::debug!("Vision VisionMgmtCommand::SwitchSz320 Received");
                    // If the command is SwitchSz320, lock the inner field and update the detector session type with Sz320
                    local_self.lock().unwrap().det.session_type =
                        detector::onnx::SessionType::Sz320;
                }
                Ok(VisionMgmtCommand::SwitchSz640) => {
                    log::debug!("Vision VisionMgmtCommand::SwitchSz640 Received");
                    // If the command is SwitchSz640, lock the inner field and update the detector session type with Sz640
                    local_self.lock().unwrap().det.session_type =
                        detector::onnx::SessionType::Sz640;
                }
                Err(_) => {} // If there is no command or an error, do nothing and proceed
            }

            // If local state is off, processing is suspended.
            if !local_state.lock().unwrap().to_owned() {
                continue;
            }

            // Send detections to other threads using the sender
            // Take an image using the camera
            {
                let mut visual_info = VisualInfo::new();
                log::debug!("Vision Camera Process Start");
                visual_info.shooting_start_time = chrono::Utc::now().timestamp_millis() as u64;
                let res_take = local_self.lock().unwrap().cam.take_picture(); // Lock the inner field and call the take method on the camera field
                visual_info.shooting_end_time = chrono::Utc::now().timestamp_millis() as u64;
                log::debug!("Vision Camera Process End");
                if res_take.is_ok() {
                    let session_type = local_self.lock().unwrap().det.session_type.clone(); // Lock the inner field and clone the session type from the detector field
                    log::debug!("Session_type:{:?}", session_type.clone());
                    let dets = local_self // Lock the inner field and call the infer method on the detector field with the image path and session type as arguments
                        .lock()
                        .unwrap()
                        .det
                        .infer(
                            &local_property.path.img.last,
                            session_type,
                            &format!(
                                "{}/{}.jpg",
                                local_property.path.dir.img, visual_info.shooting_end_time,
                            ),
                        );
                    let mut dets = dets.unwrap();
                    log::debug!("Vision Detected: {:?}", dets.clone(),);
                    // Handle ocr
                    let ocr_support = local_self.lock().unwrap().det.support_ocr();
                    if ocr_support {
                        dets = local_self
                            .lock()
                            .unwrap()
                            .det
                            .ocr(
                                &local_property.path.img.last,
                                dets.clone(),
                                local_property.as_ref().clone(),
                            )
                            .unwrap();
                        log::debug!("Vision Detected With Ocr: {:?}", dets.clone());
                    }
                    visual_info.detections = dets;
                    tx.send(visual_info).unwrap(); // Send the detection results to other threads using the sender
                }
            }
        })
    }
}

/// Visual Information
pub struct VisualInfo {
    pub shooting_start_time: u64,
    pub shooting_end_time: u64,
    pub detections: Vec<Detection>,
}

impl VisualInfo {
    pub fn new() -> Self {
        Self {
            shooting_start_time: 0,
            shooting_end_time: 0,
            detections: vec![],
        }
    }
}

impl Default for VisualInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// This struct contains the fields for the camera and the detector that are used for image processing.
pub struct RoktrackVisionInner {
    pub cam: camera::V4l2Camera, // The camera field that uses the V4l2 module
    pub det: detector::onnx::YoloV8, // The detector field that uses the YoloV8 module with onnx runtime
}

/// This impl block defines the methods for the RoktrackVisionInner struct.
impl RoktrackVisionInner {
    /// This method creates a new instance of the RoktrackVisionInner struct with the given property.
    pub fn new(property: RoktrackProperty) -> Self {
        Self {
            // Create a new camera::V4l2 instance by calling the new method on the V4l2 module and passing the property
            cam: camera::V4l2Camera::new(property.clone()),
            // Create a new detector::onnx::YoloV8 instance by calling the new method on the YoloV8 module
            det: detector::onnx::YoloV8::new(),
        }
    }
}
