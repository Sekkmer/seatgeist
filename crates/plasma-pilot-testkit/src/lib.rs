use libplasma_pilot::{CoordinateSpace, MonitorInfo};

pub fn sample_monitor() -> MonitorInfo {
    MonitorInfo {
        id: "monitor-1".to_string(),
        name: Some("Sample Monitor".to_string()),
        physical_width: 7680,
        physical_height: 4320,
        logical_width: 3840,
        logical_height: 2160,
        scale_factor: 2.0,
        logical_origin_x: 0,
        logical_origin_y: 0,
        transform: None,
    }
}

pub fn sample_coordinate_space() -> CoordinateSpace {
    CoordinateSpace::LogicalPixel
}
