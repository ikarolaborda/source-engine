//! World-to-clip transforms in Source's coordinate conventions.
//!
//! Source states positions in a right-handed world where `+x` is forward,
//! `+y` is left and `+z` is up, and states orientation as pitch/yaw/roll in
//! degrees where positive pitch looks *down*. Metal expects clip space with
//! `y` up, `x` right and `z` in `0..=w` rather than `-w..=w`. Everything that
//! reconciles those two lives here, so the rest of the renderer only ever
//! handles matrices and the engine only ever hands over the eye position and
//! angles it already tracks.

/// A 4x4 transform in the column-major order Metal's `float4x4` expects, so
/// it can be handed to a shader as bytes without being rearranged.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct Matrix(pub [[f32; 4]; 4]);

impl Matrix {
    pub const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    /// The bytes a shader reads this as.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Matrix` is `repr(C)` over sixteen `f32`, which has no
        // padding and no invalid bit patterns, so its bytes are readable.
        unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(self).cast::<u8>(),
                std::mem::size_of::<Self>(),
            )
        }
    }

    /// `self * other`, applying `other` first.
    pub fn times(&self, other: &Self) -> Self {
        let mut out = [[0.0f32; 4]; 4];
        for (column, result) in other.0.iter().zip(out.iter_mut()) {
            for (row, slot) in result.iter_mut().enumerate() {
                *slot = (0..4).map(|k| self.0[k][row] * column[k]).sum();
            }
        }
        Self(out)
    }

    /// Transforms a point, returning the clip-space coordinate including `w`.
    ///
    /// The `w` is what tells a caller whether the point is in front of the
    /// eye at all, so it is returned rather than divided out here.
    pub fn transform_point(&self, point: [f32; 3]) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for (row, slot) in out.iter_mut().enumerate() {
            *slot = self.0[0][row] * point[0]
                + self.0[1][row] * point[1]
                + self.0[2][row] * point[2]
                + self.0[3][row];
        }
        out
    }

    /// The six world-space planes bounding what this transform can show, as
    /// `[a, b, c, d]` where a point is inside the view when
    /// `a*x + b*y + c*z + d` is not negative for all six.
    ///
    /// They come out of the transform itself rather than being rebuilt from
    /// the eye and lens, so they cannot disagree with what is drawn. Each is
    /// one clip-space bound written in terms of the rows the transform
    /// applies: a point is within the left bound when its clip `x` is not
    /// less than `-w`, which is the row producing `x` added to the row
    /// producing `w`. The near bound is the row producing `z` alone, because
    /// this projection maps the near plane to zero rather than to `-w`.
    ///
    /// The order is left, right, bottom, top, near, far. Each is normalized,
    /// so `d` is a distance and the value above is how far outside a point
    /// is rather than only its sign.
    pub fn frustum_planes(&self) -> [[f32; 4]; 6] {
        let row = |index: usize| {
            [
                self.0[0][index],
                self.0[1][index],
                self.0[2][index],
                self.0[3][index],
            ]
        };
        let normalize = |plane: [f32; 4]| {
            let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            if length == 0.0 || !length.is_finite() {
                // A degenerate row cannot bound anything. A plane of zeroes
                // with a positive distance admits every point, which is the
                // safe reading: it culls nothing rather than the world away.
                return [0.0, 0.0, 0.0, 1.0];
            }
            plane.map(|value| value / length)
        };
        let add = |a: [f32; 4], b: [f32; 4]| std::array::from_fn(|index| a[index] + b[index]);
        let subtract = |a: [f32; 4], b: [f32; 4]| std::array::from_fn(|index| a[index] - b[index]);

        let (x, y, z, w) = (row(0), row(1), row(2), row(3));
        [
            normalize(add(w, x)),
            normalize(subtract(w, x)),
            normalize(add(w, y)),
            normalize(subtract(w, y)),
            normalize(z),
            normalize(subtract(w, z)),
        ]
    }
}

/// Where the camera is and which way it faces, in the terms the engine keeps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eye {
    /// Source world position, `+x` forward, `+y` left, `+z` up.
    pub position: [f32; 3],
    /// Degrees, in Source's `pitch, yaw, roll` order, where positive pitch
    /// looks down.
    pub angles: [f32; 3],
}

/// How much of the world is visible and how deep the drawable range is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lens {
    /// Horizontal field of view in degrees, which is what Source's `fov`
    /// cvar and every map's viewmodel setup are stated in.
    pub horizontal_fov: f32,
    /// Viewport width divided by height.
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

/// The eye and lens do not describe a view volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LensError {
    /// A field of view at or past a half turn has no finite projection.
    Fov(f32),
    Aspect(f32),
    /// Near and far must be positive and ordered, or depth has no range.
    Depth {
        near: f32,
        far: f32,
    },
}

impl std::fmt::Display for LensError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fov(fov) => write!(f, "field of view {fov} is not inside (0, 180) degrees"),
            Self::Aspect(aspect) => write!(f, "aspect ratio {aspect} is not positive and finite"),
            Self::Depth { near, far } => {
                write!(
                    f,
                    "depth range near {near} to far {far} is not positive and increasing"
                )
            }
        }
    }
}

impl std::error::Error for LensError {}

impl Lens {
    /// The projection from Source eye space to Metal clip space.
    fn projection(&self) -> Result<Matrix, LensError> {
        if !self.horizontal_fov.is_finite()
            || self.horizontal_fov <= 0.0
            || self.horizontal_fov >= 180.0
        {
            return Err(LensError::Fov(self.horizontal_fov));
        }
        if !self.aspect.is_finite() || self.aspect <= 0.0 {
            return Err(LensError::Aspect(self.aspect));
        }
        if !self.near.is_finite()
            || !self.far.is_finite()
            || self.near <= 0.0
            || self.far <= self.near
        {
            return Err(LensError::Depth {
                near: self.near,
                far: self.far,
            });
        }

        let half_x = (self.horizontal_fov.to_radians() / 2.0).tan();
        let half_y = half_x / self.aspect;

        // Metal maps the near plane to z = 0 and the far plane to z = w,
        // unlike OpenGL's -w..w, so the depth row is scaled and biased for
        // that range rather than the one ToGL's shaders were built for.
        let depth_scale = self.far / (self.far - self.near);
        Ok(Matrix([
            [1.0 / half_x, 0.0, 0.0, 0.0],
            [0.0, 1.0 / half_y, 0.0, 0.0],
            [0.0, 0.0, depth_scale, 1.0],
            [0.0, 0.0, -self.near * depth_scale, 0.0],
        ]))
    }
}

/// The world-to-clip transform for an eye looking through a lens.
pub fn view_projection(eye: Eye, lens: Lens) -> Result<Matrix, LensError> {
    Ok(lens.projection()?.times(&view(eye)))
}

/// The world-to-eye transform, which rotates Source's axes onto the ones the
/// projection above is stated in and then moves the world under the eye.
fn view(eye: Eye) -> Matrix {
    let (pitch, yaw, roll) = (
        eye.angles[0].to_radians(),
        eye.angles[1].to_radians(),
        eye.angles[2].to_radians(),
    );
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let (sr, cr) = roll.sin_cos();

    // Source's forward/right/up for these angles. Positive pitch looks down,
    // which is why forward's z term is negated relative to the usual form.
    let forward = [cp * cy, cp * sy, -sp];
    let right = [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp];
    let up = [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp];

    // Eye space puts `x` right, `y` up and `z` along the view direction, so
    // each basis vector becomes a row of the rotation and the translation is
    // the eye position projected onto them.
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let position = eye.position;
    Matrix([
        [right[0], up[0], forward[0], 0.0],
        [right[1], up[1], forward[1], 0.0],
        [right[2], up[2], forward[2], 0.0],
        [
            -dot(right, position),
            -dot(up, position),
            -dot(forward, position),
            1.0,
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LENS: Lens = Lens {
        horizontal_fov: 90.0,
        aspect: 1.0,
        near: 1.0,
        far: 1000.0,
    };

    fn eye_at_origin() -> Eye {
        Eye {
            position: [0.0, 0.0, 0.0],
            angles: [0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn puts_what_the_camera_faces_in_the_middle_of_the_screen() {
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");

        // Straight ahead in Source is +x, ten units out.
        let clip = transform.transform_point([10.0, 0.0, 0.0]);

        assert!(clip[3] > 0.0, "a point ahead of the eye is in front of it");
        let screen_x = clip[0] / clip[3];
        let screen_y = clip[1] / clip[3];
        assert!(
            screen_x.abs() < 1e-5,
            "centred horizontally, got {screen_x}"
        );
        assert!(screen_y.abs() < 1e-5, "centred vertically, got {screen_y}");
    }

    #[test]
    fn puts_the_world_the_way_round_the_screen_is() {
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");

        // Source's +y is to the camera's left, and +z is up.
        let left = transform.transform_point([10.0, 10.0, 0.0]);
        let up = transform.transform_point([10.0, 0.0, 10.0]);

        assert!(
            left[0] / left[3] < 0.0,
            "the world's left is the screen's left"
        );
        assert!(up[1] / up[3] > 0.0, "the world's up is the screen's up");
    }

    #[test]
    fn fills_the_screen_at_the_edges_of_the_field_of_view() {
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");

        // At 90 degrees horizontally, the edge of view is 45 degrees off
        // forward, so a point the same distance out as it is to the side sits
        // exactly on the screen edge.
        let edge = transform.transform_point([10.0, -10.0, 0.0]);

        let screen_x = edge[0] / edge[3];
        assert!(
            (screen_x - 1.0).abs() < 1e-5,
            "the edge of the field of view is the edge of the screen, got {screen_x}"
        );
    }

    #[test]
    fn maps_the_depth_range_onto_the_one_metal_reads() {
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");

        let near = transform.transform_point([LENS.near, 0.0, 0.0]);
        let far = transform.transform_point([LENS.far, 0.0, 0.0]);

        assert!(
            (near[2] / near[3]).abs() < 1e-5,
            "the near plane is depth zero"
        );
        assert!(
            (far[2] / far[3] - 1.0).abs() < 1e-4,
            "the far plane is depth one"
        );
    }

    #[test]
    fn puts_what_is_behind_the_camera_behind_it() {
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");

        let behind = transform.transform_point([-10.0, 0.0, 0.0]);

        assert!(
            behind[3] < 0.0,
            "a point behind the eye has a negative w so it clips away"
        );
    }

    #[test]
    fn turns_with_the_camera() {
        // Yawing 90 degrees turns the camera to face Source's +y, its left.
        let turned = Eye {
            position: [0.0, 0.0, 0.0],
            angles: [0.0, 90.0, 0.0],
        };
        let transform = view_projection(turned, LENS).expect("a 90 degree square lens");

        let clip = transform.transform_point([0.0, 10.0, 0.0]);

        assert!(clip[3] > 0.0, "what the camera turned towards is ahead");
        assert!(
            (clip[0] / clip[3]).abs() < 1e-5,
            "and it is centred once turned"
        );
    }

    #[test]
    fn looks_down_when_the_pitch_is_positive() {
        let pitched = Eye {
            position: [0.0, 0.0, 0.0],
            angles: [45.0, 0.0, 0.0],
        };
        let transform = view_projection(pitched, LENS).expect("a 90 degree square lens");

        // Source's positive pitch looks down, so the point below the eye is
        // the one that ends up centred.
        let below = transform.transform_point([10.0, 0.0, -10.0]);

        assert!(
            below[3] > 0.0,
            "the ground ahead is in view when looking down"
        );
        assert!(
            (below[1] / below[3]).abs() < 1e-5,
            "and it is centred at 45 degrees of pitch"
        );
    }

    #[test]
    fn moves_with_the_camera() {
        let moved = Eye {
            position: [100.0, 0.0, 0.0],
            angles: [0.0, 0.0, 0.0],
        };
        let transform = view_projection(moved, LENS).expect("a 90 degree square lens");

        let behind_the_new_position = transform.transform_point([50.0, 0.0, 0.0]);
        let ahead_of_it = transform.transform_point([150.0, 0.0, 0.0]);

        assert!(
            behind_the_new_position[3] < 0.0,
            "what the camera has passed is behind it"
        );
        assert!(ahead_of_it[3] > 0.0, "what it has not reached is ahead");
    }

    #[test]
    fn refuses_a_lens_that_does_not_describe_a_view_volume() {
        let cases = [
            (
                Lens {
                    horizontal_fov: 180.0,
                    ..LENS
                },
                LensError::Fov(180.0),
            ),
            (
                Lens {
                    horizontal_fov: 0.0,
                    ..LENS
                },
                LensError::Fov(0.0),
            ),
            (
                Lens {
                    aspect: 0.0,
                    ..LENS
                },
                LensError::Aspect(0.0),
            ),
            (
                Lens { near: 0.0, ..LENS },
                LensError::Depth {
                    near: 0.0,
                    far: 1000.0,
                },
            ),
            (
                Lens {
                    near: 10.0,
                    far: 10.0,
                    ..LENS
                },
                LensError::Depth {
                    near: 10.0,
                    far: 10.0,
                },
            ),
        ];

        for (lens, expected) in cases {
            assert_eq!(
                view_projection(eye_at_origin(), lens).err(),
                Some(expected),
                "{lens:?} does not describe a view volume"
            );
        }
        assert!(
            view_projection(
                eye_at_origin(),
                Lens {
                    aspect: f32::NAN,
                    ..LENS
                }
            )
            .is_err(),
            "an aspect that is not a number does not describe a view volume"
        );
    }

    /// How far inside a frustum a point is, as the least of its distances
    /// from the six planes: positive inside, negative outside.
    fn inside_by(planes: &[[f32; 4]; 6], point: [f32; 3]) -> f32 {
        planes
            .iter()
            .map(|plane| plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] + plane[3])
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn bounds_the_view_with_the_planes_the_transform_produces() {
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");
        let planes = transform.frustum_planes();

        // The eye looks along `+x`, so a point down that axis between the
        // near and far planes is inside and one behind the eye is not.
        assert!(inside_by(&planes, [100.0, 0.0, 0.0]) > 0.0);
        assert!(inside_by(&planes, [-10.0, 0.0, 0.0]) < 0.0);
        // Nearer than the near plane and further than the far plane are both
        // outside, which is what makes these six planes rather than four.
        assert!(inside_by(&planes, [LENS.near / 2.0, 0.0, 0.0]) < 0.0);
        assert!(inside_by(&planes, [LENS.far * 2.0, 0.0, 0.0]) < 0.0);
        // At a 90 degree horizontal field of view the view's edge is the
        // diagonal, so a point as far to the left as it is forward is on it.
        assert!(inside_by(&planes, [100.0, 99.0, 0.0]) > 0.0);
        assert!(inside_by(&planes, [100.0, 101.0, 0.0]) < 0.0);
        assert!(inside_by(&planes, [100.0, -101.0, 0.0]) < 0.0);
        // The lens is square, so the same holds above and below.
        assert!(inside_by(&planes, [100.0, 0.0, 101.0]) < 0.0);
        assert!(inside_by(&planes, [100.0, 0.0, -101.0]) < 0.0);
    }

    #[test]
    fn states_the_bounding_planes_as_distances() {
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");
        let planes = transform.frustum_planes();

        for plane in &planes {
            let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            assert!(
                (length - 1.0).abs() < 1e-5,
                "a normalized plane's value is how far outside a point is, got a normal of {length}"
            );
        }
        // The near plane faces along the eye's forward axis and sits at the
        // near distance, so a point on it is at no distance from it.
        let near = planes[4];
        assert!((near[0] - 1.0).abs() < 1e-5, "the near plane faces forward");
        let on_it = near[0] * LENS.near + near[1] * 0.0 + near[2] * 0.0 + near[3];
        assert!(
            on_it.abs() < 1e-4,
            "a point on the near plane is on it, got {on_it}"
        );
    }

    #[test]
    fn bounds_the_view_the_camera_is_actually_looking_through() {
        // The planes have to turn and move with the camera, or culling by
        // them would remove what is on screen as soon as the view changed.
        let eye = Eye {
            position: [500.0, -200.0, 64.0],
            angles: [0.0, 90.0, 0.0],
        };
        let transform = view_projection(eye, LENS).expect("a 90 degree square lens");
        let planes = transform.frustum_planes();

        // Yaw of ninety turns the eye to face along `+y`.
        assert!(inside_by(&planes, [500.0, -100.0, 64.0]) > 0.0);
        assert!(inside_by(&planes, [500.0, -300.0, 64.0]) < 0.0);
        // And what was ahead before the turn is now to the side, past the
        // diagonal the field of view reaches.
        assert!(inside_by(&planes, [700.0, -200.0, 64.0]) < 0.0);
    }

    #[test]
    fn composes_in_the_order_the_transforms_apply() {
        let identity = Matrix::IDENTITY;
        let transform = view_projection(eye_at_origin(), LENS).expect("a 90 degree square lens");

        assert_eq!(transform.times(&identity), transform);
        assert_eq!(identity.times(&transform), transform);
    }

    #[test]
    fn hands_a_shader_sixteen_floats_in_column_order() {
        let matrix = Matrix([
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
            [13.0, 14.0, 15.0, 16.0],
        ]);

        let bytes = matrix.as_bytes();

        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[0..4], &1.0f32.to_ne_bytes());
        assert_eq!(&bytes[4..8], &2.0f32.to_ne_bytes());
        assert_eq!(&bytes[60..64], &16.0f32.to_ne_bytes());
    }
}
