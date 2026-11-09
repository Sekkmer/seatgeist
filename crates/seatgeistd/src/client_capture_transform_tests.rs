use super::*;
use crate::compatibility_capture_backend::{OneShotCaptureSession, OneShotFrameCapture};
use async_trait::async_trait;
use seatgeist_backend::{CaptureSessionMetadata, CaptureSourceType};

#[derive(Debug)]
struct ClientCapture {
    native: (u32, u32),
    logical: (u32, u32),
    output: (u32, u32),
}

#[async_trait]
impl OneShotFrameCapture for ClientCapture {
    async fn capture(&self, request: &FrameRequest) -> seatgeist_backend::Result<ScreenshotInfo> {
        std::fs::write(&request.output, b"retained client capture fixture").unwrap();
        Ok(ScreenshotInfo {
            path: request.output.clone().into(),
            backend: "kwin_screenshot2_window".into(),
            occlusion_possible: false,
            source_width: self.native.0,
            source_height: self.native.1,
            output_width: self.output.0,
            output_height: self.output.1,
            coordinate_space: CoordinateSpace::WindowLocal,
            transform: ScreenshotTransform {
                source_coordinate_space: CoordinateSpace::WindowLocal,
                output_coordinate_space: CoordinateSpace::CaptureOutput,
                source_extent_width: Some(self.logical.0),
                source_extent_height: Some(self.logical.1),
                source_origin_x: 0,
                source_origin_y: 0,
                scale_x: f64::from(self.output.0) / f64::from(self.logical.0),
                scale_y: f64::from(self.output.1) / f64::from(self.logical.1),
            },
            monitors: vec![],
        })
    }
}

#[tokio::test]
async fn exact_client_extent_survives_one_shot_retention_without_monitor_metadata() {
    for (native, logical, output) in [
        ((1280, 720), (1280, 720), (1280, 720)),
        ((1920, 1080), (1280, 720), (1280, 720)),
        ((1920, 1080), (1280, 720), (640, 360)),
    ] {
        let id = format!("client-transform-{}", Uuid::new_v4());
        let output_path = std::env::temp_dir().join(format!("{id}.png"));
        let store = CaptureSessionStore::default();
        store
            .install(
                Box::new(OneShotCaptureSession::new(
                    CaptureSessionMetadata {
                        id: id.clone(),
                        backend: "kwin_screenshot2_window".into(),
                        source_type: CaptureSourceType::Window,
                        source_id: Some("window-client".into()),
                        restore_token_reference: None,
                        consent_required: false,
                        occlusion_possible: false,
                    },
                    Arc::new(ClientCapture {
                        native,
                        logical,
                        output,
                    }),
                )),
                Some("window-client".into()),
            )
            .await;
        let mut frame = store
            .snapshot(CaptureSnapshotRequest {
                session_id: id.clone(),
                output: output_path.clone(),
                max_edge: Some(output.0),
                timeout_ms: 5000,
            })
            .await
            .unwrap();
        let mut window = seatgeist_testkit::sample_window();
        window.id = "window-client".into();
        window.monitor_id = None;
        window.geometry = Some(libseatgeist::WindowGeometry {
            x: 320,
            y: 166,
            width: 1280,
            height: 748,
            space: CoordinateSpace::LogicalPixel,
        });
        let windows = seatgeist_testkit::MockWindowBackend::new(vec![window], None);
        let screens = seatgeist_testkit::MockScreenBackend::with_frames(vec![], vec![]);
        let status = store.status_for_session(&id).await;
        crate::annotate_exact_window_capture_transform(&status, &mut frame, &windows, &screens)
            .await
            .unwrap();
        let missing_window = seatgeist_testkit::MockWindowBackend::new(vec![], None);
        assert!(
            crate::annotate_exact_window_capture_transform(
                &status,
                &mut frame,
                &missing_window,
                &screens
            )
            .await
            .is_err()
        );
        assert_eq!(frame.screenshot.source_width, native.0);
        assert_eq!(frame.screenshot.source_height, native.1);
        assert_eq!(frame.screenshot.transform.source_extent_height, Some(720));
        store.update_latest_frame(&frame).await.unwrap();
        for (x, y) in [
            (177.0, 10.0),
            (935.0, 698.0),
            (640.0, 360.0),
            (1279.0, 719.0),
        ] {
            let factor = f64::from(output.0) / 1280.0;
            let point = store
                .resolve_capture_output_point(
                    &id,
                    &frame.revision,
                    libseatgeist::Point {
                        x: x * factor,
                        y: y * factor,
                        space: CoordinateSpace::CaptureOutput,
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                (point.x, point.y, point.space),
                (x, y, CoordinateSpace::WindowLocal)
            );
        }
        let point = libseatgeist::Point {
            x: 1.0,
            y: 1.0,
            space: CoordinateSpace::CaptureOutput,
        };
        assert!(
            store
                .resolve_capture_output_point(&id, "old-revision", point)
                .await
                .is_err()
        );
        assert!(
            store
                .resolve_capture_output_point(
                    &id,
                    &frame.revision,
                    libseatgeist::Point {
                        x: f64::from(output.0),
                        y: 1.0,
                        space: CoordinateSpace::CaptureOutput,
                    }
                )
                .await
                .is_err()
        );
        // A replacement session cannot consume the old session's revision.
        store
            .close(libseatgeist::CaptureSessionRequest {
                session_id: id.clone(),
            })
            .await
            .unwrap();
        assert!(
            store
                .resolve_capture_output_point(&id, &frame.revision, point)
                .await
                .is_err()
        );
        let mut invalid = frame;
        invalid.screenshot.transform.source_extent_height = None;
        assert!(crate::preserve_authoritative_client_transform(&invalid).is_err());
        std::fs::remove_file(output_path).unwrap();
    }
}
