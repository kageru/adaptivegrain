use super::PLUGIN_NAME;
use failure::Error;
use std::ptr;
use vapoursynth::core::CoreRef;
use vapoursynth::format::ColorFamily;
use vapoursynth::plugins::{Filter, FrameContext};
use vapoursynth::prelude::*;
use vapoursynth::video_info::{Property, VideoInfo};

pub struct Mask<'core> {
    pub source: Node<'core>,
    pub luma_scaling: f32,
}

lazy_static! {
    pub static ref FLOAT_RANGE: Vec<f32> = (0..256).map(|i| (i as f32) / 256.0).collect();
}

#[inline]
pub fn get_mask_value(x: f32, luma_scaling: f32) -> f32 {
    f32::powf(
        1.0 - (x
            * (x.mul_add(
                x.mul_add(x.mul_add(x.mul_add(18.188, -45.47), 36.624), -9.466),
                1.124,
            ))),
        luma_scaling,
    )
}

#[inline]
pub fn get_mask_value_clamping(x: f32, luma_scaling: f32) -> f32 {
    get_mask_value(x.min(1.0).max(0.0), luma_scaling)
}

macro_rules! from_property {
    ($prop: expr) => {
        match $prop {
            Property::Constant(p) => p,
            Property::Variable => unreachable!(),
        }
    };
}

macro_rules! int_filter {
    ($type:ty, $fname:ident) => {
        fn $fname(frame: &mut FrameRefMut, src_frame: FrameRef, depth: u8, luma_scaling: f32) {
            let max = ((1 << depth) - 1) as f32;
            let lut: Vec<$type> = FLOAT_RANGE
                .iter()
                .map(|x| (get_mask_value(*x, luma_scaling) * max) as $type)
                .collect();
            for row in 0..frame.height(0) {
                for (pixel, src_pixel) in frame
                    .plane_row_mut::<$type>(0, row)
                    .iter_mut()
                    .zip(src_frame.plane_row::<$type>(0, row))
                {
                    let i = (src_pixel >> (depth - 8)) as usize;
                    unsafe {
                        ptr::write(pixel, lut[i].clone());
                    }
                }
            }
        }
    };
}

fn filter_for_float(frame: &mut FrameRefMut, src_frame: FrameRef, luma_scaling: f32) {
    for row in 0..frame.height(0) {
        frame
            .plane_row_mut::<f32>(0, row)
            .iter_mut()
            .zip(src_frame.plane_row::<f32>(0, row))
            .for_each(|(pixel, src_pixel)| unsafe {
                ptr::write(pixel, get_mask_value(*src_pixel, luma_scaling));
            });
    }
}

fn filter_for_float_clamping(frame: &mut FrameRefMut, src_frame: FrameRef, luma_scaling: f32) {
    for row in 0..frame.height(0) {
        frame
            .plane_row_mut::<f32>(0, row)
            .iter_mut()
            .zip(src_frame.plane_row::<f32>(0, row))
            .for_each(|(pixel, src_pixel)| unsafe {
                ptr::write(pixel, get_mask_value_clamping(*src_pixel, luma_scaling));
            });
    }
}

impl<'core> Filter<'core> for Mask<'core> {
    fn video_info(&self, _api: API, _core: CoreRef<'core>) -> Vec<VideoInfo<'core>> {
        let info = self.source.info();
        let format = match info.format {
            Property::Variable => unreachable!(),
            Property::Constant(format) => format,
        };
        vec![VideoInfo {
            format: Property::Constant(
                _core
                    .register_format(
                        ColorFamily::Gray,
                        format.sample_type(),
                        format.bits_per_sample(),
                        0,
                        0,
                    )
                    .unwrap(),
            ),
            flags: info.flags,
            framerate: info.framerate,
            num_frames: info.num_frames,
            resolution: info.resolution,
        }]
    }

    fn get_frame_initial(
        &self,
        _api: API,
        _core: CoreRef<'core>,
        context: FrameContext,
        n: usize,
    ) -> Result<Option<FrameRef<'core>>, Error> {
        self.source.request_frame_filter(context, n);
        Ok(None)
    }

    fn get_frame(
        &self,
        _api: API,
        core: CoreRef<'core>,
        context: FrameContext,
        n: usize,
    ) -> Result<FrameRef<'core>, Error> {
        let new_format = from_property!(self.video_info(_api, core)[0].format);
        let mut frame = unsafe {
            FrameRefMut::new_uninitialized(
                core,
                None,
                new_format,
                from_property!(self.source.info().resolution),
            )
        };
        let src_frame = self.source.get_frame_filter(context, n).ok_or_else(|| {
            format_err!("Could not retrieve source frame. This shouldn’t happen.")
        })?;
        let props = src_frame.props();
        let average = match props.get::<f64>("PlaneStatsAverage") {
            Ok(average) => average as f32,
            Err(_) => bail!(format!(
                "{}: you need to run std.PlaneStats on the clip before calling this function.",
                PLUGIN_NAME
            )),
        };

        match from_property!(self.source.info().format).sample_type() {
            SampleType::Integer => {
                let depth = from_property!(self.source.info().format).bits_per_sample();
                match depth {
                    0..=8 => {
                        int_filter!(u8, filter_8bit);
                        filter_8bit(
                            &mut frame,
                            src_frame,
                            depth,
                            calc_luma_scaling(average, self.luma_scaling),
                        )
                    }
                    9..=16 => {
                        int_filter!(u16, filter_16bit);
                        filter_16bit(
                            &mut frame,
                            src_frame,
                            depth,
                            calc_luma_scaling(average, self.luma_scaling),
                        )
                    }
                    17..=32 => {
                        int_filter!(u32, filter_32bit);
                        filter_32bit(
                            &mut frame,
                            src_frame,
                            depth,
                            calc_luma_scaling(average, self.luma_scaling),
                        )
                    }
                    _ => bail!(format!(
                        "{}: input depth {} not supported",
                        PLUGIN_NAME, depth
                    )),
                }
            }
            SampleType::Float => {
                // If the input has pixel values outside of the valid range (0-1),
                // those might also be out of range in the output.
                // We use the min/max props to determine if output clamping is necessary.
                let max = props
                    .get::<f64>("PlaneStatsMax")
                    .expect(&format!("{}: no PlaneStatsMax in frame props", PLUGIN_NAME));
                let min = props
                    .get::<f64>("PlaneStatsMin")
                    .expect(&format!("{}: no PlaneStatsMin in frame props", PLUGIN_NAME));
                if max > 1.0 || min < 0.0 {
                    filter_for_float_clamping(
                        &mut frame,
                        src_frame,
                        calc_luma_scaling(average, self.luma_scaling),
                    );
                } else {
                    filter_for_float(
                        &mut frame,
                        src_frame,
                        calc_luma_scaling(average, self.luma_scaling),
                    );
                }
            }
        }
        Ok(frame.into())
    }
}

pub fn calc_luma_scaling(average: f32, luma_scaling: f32) -> f32 {
    let average = average.min(1.0).max(0.0);
    average * average * luma_scaling
}

#[cfg(test)]
mod tests {
    use super::*;

    // Just in case this isn’t the last time I rewrite the lut builder:
    #[rustfmt::skip]
    static EXPECTED_MASK_02: [f32; 256] = [1.0, 0.99829847, 0.99670357, 0.9952108, 0.99381596, 0.99251467, 0.9913026, 0.99017555, 0.98912925, 0.9881595, 0.9872621, 0.98643315, 0.98566836, 0.98496383, 0.98431563, 0.98371994, 0.9831729, 0.9826707, 0.98220986, 0.9817866, 0.9813975, 0.9810391, 0.980708, 0.980401, 0.9801147, 0.9798461, 0.97959214, 0.97934985, 0.9791162, 0.9788885, 0.9786639, 0.97843987, 0.9782137, 0.97798294, 0.97774506, 0.9774978, 0.9772388, 0.97696584, 0.9766768, 0.97636956, 0.9760422, 0.97569263, 0.97531915, 0.97491974, 0.97449285, 0.9740367, 0.9735496, 0.9730301, 0.9724766, 0.9718877, 0.97126204, 0.9705982, 0.96989495, 0.96915096, 0.9683652, 0.9675364, 0.9666635, 0.9657455, 0.96478134, 0.9637701, 0.96271086, 0.96160275, 0.9604449, 0.9592366, 0.95797706, 0.9566655, 0.9553014, 0.95388395, 0.95241266, 0.95088685, 0.949306, 0.9476697, 0.9459774, 0.9442286, 0.9424229, 0.94056, 0.93863946, 0.93666095, 0.93462414, 0.9325288, 0.93037456, 0.9281613, 0.9258888, 0.9235568, 0.92116517, 0.9187138, 0.9162025, 0.9136312, 0.91099983, 0.9083083, 0.9055567, 0.90274477, 0.8998728, 0.8969405, 0.8939482, 0.8908958, 0.88778335, 0.88461095, 0.8813788, 0.87808704, 0.87473565, 0.871325, 0.8678552, 0.8643263, 0.8607387, 0.8570925, 0.85338813, 0.8496256, 0.84580535, 0.8419277, 0.83799297, 0.83400136, 0.8299533, 0.8258492, 0.8216893, 0.8174741, 0.81320417, 0.80887955, 0.8045011, 0.80006903, 0.79558396, 0.7910463, 0.78645676, 0.7818155, 0.7771236, 0.7723811, 0.76758915, 0.7627478, 0.75785816, 0.75292087, 0.7479362, 0.74290544, 0.7378288, 0.7327075, 0.72754174, 0.72233313, 0.71708167, 0.7117891, 0.7064553, 0.7010822, 0.6956699, 0.69022, 0.6847333, 0.6792108, 0.67365366, 0.668063, 0.6624399, 0.65678567, 0.65110123, 0.6453887, 0.63964826, 0.63388246, 0.6280915, 0.622278, 0.61644244, 0.6105874, 0.6047135, 0.5988235, 0.5929182, 0.58700025, 0.58107054, 0.57513213, 0.5691858, 0.56323487, 0.55728036, 0.5513258, 0.54537195, 0.5394228, 0.5334794, 0.52754575, 0.52162296, 0.5157146, 0.50982434, 0.5039534, 0.49810678, 0.4922856, 0.48649472, 0.48073623, 0.47501534, 0.46933344, 0.46369645, 0.4581058, 0.45256722, 0.44708318, 0.4416599, 0.4362986, 0.4310061, 0.42578426, 0.42064053, 0.41557512, 0.41059688, 0.4057069, 0.4009107, 0.39621186, 0.39161587, 0.3871265, 0.38274762, 0.37848303, 0.37433672, 0.37031212, 0.36641234, 0.36264044, 0.35899892, 0.35549006, 0.35211465, 0.34887564, 0.3457732, 0.34280726, 0.33997762, 0.33728316, 0.3347223, 0.33229122, 0.32998928, 0.3278117, 0.32575342, 0.32380822, 0.32197225, 0.32023716, 0.31859392, 0.31703717, 0.3155556, 0.31413817, 0.31277677, 0.31145856, 0.3101692, 0.3088991, 0.30763134, 0.30635372, 0.30504793, 0.30370083, 0.30229148, 0.30080518, 0.29921892, 0.29751524, 0.29566884, 0.29365554, 0.29145348, 0.28903463, 0.28636503, 0.28341296, 0.2801431, 0.27650893, 0.27246484, 0.2679581, 0.26291874, 0.25727344, 0.25092778, 0.24376981, 0.23564698, 0.22636926, 0.2156733, 0.20318091, 0.18831593, 0.17008513, 0.14654778, 0.11254175];
    #[rustfmt::skip]
    static EXPECTED_MASK_08: [f32; 256] = [1.0, 0.97312033, 0.94854075, 0.92606485, 0.9055145, 0.88672876, 0.86956084, 0.8538767, 0.839554, 0.82648087, 0.8145538, 0.8036785, 0.793767, 0.7847378, 0.7765157, 0.7690307, 0.76221645, 0.7560116, 0.7503583, 0.7452015, 0.7404903, 0.73617536, 0.73221, 0.72855055, 0.7251544, 0.72198164, 0.7189933, 0.71615267, 0.7134242, 0.7107743, 0.70816976, 0.7055801, 0.7029752, 0.7003262, 0.69760597, 0.6947885, 0.69184875, 0.68876356, 0.6855104, 0.68206835, 0.6784181, 0.6745413, 0.6704213, 0.66604257, 0.6613911, 0.65645486, 0.65122217, 0.6456841, 0.6398328, 0.6336617, 0.6271661, 0.6203427, 0.6131898, 0.6057077, 0.5978976, 0.5897622, 0.5813064, 0.57253623, 0.56345886, 0.55408335, 0.54441965, 0.5344795, 0.52427536, 0.5138211, 0.5031317, 0.4922232, 0.48111236, 0.46981704, 0.45835546, 0.4467467, 0.4350106, 0.42316702, 0.4112368, 0.39924026, 0.38719845, 0.37513202, 0.36306223, 0.35100925, 0.33899403, 0.32703626, 0.31515577, 0.30337203, 0.29170325, 0.28016755, 0.268782, 0.25756323, 0.2465265, 0.23568656, 0.22505718, 0.21465099, 0.20447965, 0.19455387, 0.18488336, 0.17547633, 0.1663404, 0.15748179, 0.14890584, 0.14061676, 0.13261788, 0.1249111, 0.11749773, 0.11037807, 0.10355142, 0.09701606, 0.09076972, 0.08480923, 0.07913076, 0.07372954, 0.06860047, 0.06373775, 0.05913521, 0.05478584, 0.05068255, 0.046817873, 0.043183923, 0.039772622, 0.03657578, 0.03358472, 0.030791199, 0.0281864, 0.025761817, 0.02350881, 0.021418924, 0.019483581, 0.017694633, 0.01604377, 0.014523158, 0.013124882, 0.011841504, 0.010665701, 0.009590316, 0.008608681, 0.0077141733, 0.0069006504, 0.0061620683, 0.005492886, 0.0048876274, 0.004341311, 0.0038490645, 0.003406462, 0.0030092031, 0.002653386, 0.0023353002, 0.0020515076, 0.0017988166, 0.0015742666, 0.0013751301, 0.0011988885, 0.0010432207, 0.0009060293, 0.0007853431, 0.0006794224, 0.000586633, 0.0005055344, 0.00043479103, 0.00037322083, 0.00031973835, 0.000273389, 0.00023330118, 0.00019870678, 0.00016891248, 0.00014331177, 0.00012135691, 0.00010257295, 0.00008653258, 0.00007286733, 0.000061247025, 0.000051388823, 0.00004304101, 0.000035988374, 0.0000300405, 0.000025035166, 0.000020831892, 0.000017307977, 0.000014360154, 0.000011898005, 0.000009845784, 0.000008137849, 0.0000067192714, 0.0000055424794, 0.0000045681495, 0.0000037622924, 0.000003096917, 0.0000025480817, 0.000002096078, 0.0000017240154, 0.0000014181916, 0.0000011669001, 0.0000009606775, 0.0000007913941, 0.0000006526048, 0.00000053876414, 0.00000044541238, 0.0000003688414, 0.0000003060302, 0.0000002544758, 0.0000002121291, 0.00000017731381, 0.0000001486616, 0.00000012505093, 0.00000010556544, 0.00000008945813, 0.000000076118496, 0.00000006504881, 0.000000055840008, 0.000000048164882, 0.000000041750454, 0.000000036374765, 0.000000031857045, 0.00000002804884, 0.000000024828736, 0.0000000220954, 0.000000019769583, 0.000000017782428, 0.000000016077687, 0.000000014608518, 0.000000013338133, 0.000000012233408, 0.000000011266786, 0.00000001041748, 0.000000009665278, 0.000000008993553, 0.000000008389807, 0.000000007841596, 0.000000007338018, 0.0000000068717365, 0.0000000064341195, 0.000000006019636, 0.0000000056219833, 0.000000005237641, 0.0000000048619975, 0.0000000044932946, 0.000000004128811, 0.0000000037683177, 0.0000000034110592, 0.0000000030578147, 0.000000002710866, 0.0000000023724578, 0.0000000020451252, 0.0000000017326667, 0.0000000014390599, 0.0000000011677407, 0.00000000092250246, 0.00000000070643236, 0.00000000052136806, 0.00000000036839945, 0.00000000024704733, 0.00000000015548153, 0.0000000000904045, 0.000000000047542185, 0.000000000021915018, 0.000000000008435807, 0.0000000000025014576, 0.0000000000004905257, 0.00000000000004525513, 0.0000000000000006622447];

    #[test]
    fn test_mask_values() {
        FLOAT_RANGE
            .iter()
            .zip(EXPECTED_MASK_02.iter())
            .for_each(|(&x, &exp)| {
                assert!((get_mask_value(x, calc_luma_scaling(0.2, 10.0)) - exp).abs() < 0.0001);
            });
        FLOAT_RANGE
            .iter()
            .zip(EXPECTED_MASK_08.iter())
            .for_each(|(&x, &exp)| {
                assert!((get_mask_value(x, calc_luma_scaling(0.8, 10.0)) - exp).abs() < 0.0001);
            });
    }

    #[test]
    fn test_mask_values_clamping() {
        FLOAT_RANGE
            .iter()
            .zip(EXPECTED_MASK_02.iter())
            .for_each(|(&x, &exp)| {
                assert!(
                    (get_mask_value_clamping(x, calc_luma_scaling(0.2, 10.0)) - exp).abs() < 0.0001
                );
            });
        assert_eq!(
            get_mask_value_clamping(1.1, calc_luma_scaling(0.99, 10.0)),
            0.0
        );
        assert_eq!(
            get_mask_value_clamping(-0.1, calc_luma_scaling(-0.1, 10.0)),
            1.0
        );
    }
}
