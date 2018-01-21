//! **Spherical Cow**: *A high volume fraction sphere packing library*.
//!
//! # Usage
//!
//! First, add `spherical-cow` to the dependencies in your project's `Cargo.toml`.
//!
//! ```toml
//! [dependencies]
//! spherical-cow = "0.1"
//! ```
//!
//! And this in your crate root:
//!
//! ```rust
//! extern crate spherical_cow;
//! ```
//!
//! Currently this library requires the rust nightly compiler as at depends on the `remove_item` function of `Vec`.
//!
//! To calculate the `volume_fraction` of a spherical container with radius 2 filled with spheres of radii between 0.05 and 0.1 is straightforward:
//!
//! ```rust,no_run
//! extern crate nalgebra;
//! extern crate rand;
//! extern crate spherical_cow;
//!
//! use spherical_cow::shapes::Sphere;
//! use spherical_cow::PackedVolume;
//! use rand::distributions::Range;
//! use nalgebra::Point3;
//!
//! fn main() {
//!     // Pack spheres with radii between 0.05 and 0.1 into a spherical container of radius 2,
//!     // output quantitative analysis data.
//!     let boundary = Sphere::new(Point3::origin(), 2.0);
//!     let mut sizes = Range::new(0.05, 0.1);
//!
//!     let packed = PackedVolume::new(boundary, &mut sizes);
//!
//!     println!("Volume Fraction: {:.2}%", packed.volume_fraction() * 100.);
//! }
//! ```
//!
//! A full list of examples can be found in the [examples](https://github.com/Libbum/spherical-cow/tree/master/examples) directory.
//!
//! # Research
//!
//! The method implemented herein is an advancing front algorithm from
//! Valera *et al.*, [Computational Particle Mechanics 2, 161 (2015)](https://doi.org/10.1007/s40571-015-0045-8).

#![cfg_attr(feature = "dev", feature(plugin))]
#![cfg_attr(feature = "dev", plugin(clippy))]
#![warn(missing_docs)]
#![feature(vec_remove_item)]

extern crate nalgebra;
extern crate rand;
#[cfg(feature = "serde-1")]
extern crate serde;

pub mod shapes;
pub mod util;
#[cfg(feature = "serde-1")]
mod serialization;

use nalgebra::Point3;
use nalgebra::core::{Matrix, Matrix3};
use rand::Rng;
use rand::distributions::IndependentSample;
use std::iter::repeat;
use shapes::Sphere;

/// The `Container` trait must be implemented for all shapes you wish to pack spheres into.
/// Standard shapes such as spheres and cuboids already derrive this trait. More complicated
/// shapes such as a triangular mesh are also straightforward to implement, examples
/// of such can be seen in the
/// [show_in_emerald](https://github.com/Libbum/spherical-cow/blob/master/examples/show_in_emerald.rs)
/// and [show_in_cow](https://github.com/Libbum/spherical-cow/blob/master/examples/show_in_cow.rs) files.
pub trait Container {
    /// Checks if a sphere exists inside some bounding geometry.
    fn contains(&self, sphere: &Sphere) -> bool;
    /// Calculates the volume of this container in normalised units.
    fn volume(&self) -> f32;
}

/// To obtain quantitative values of your packing effectiveness, `PackedVolume` provides
/// a number of useful indicators of such.
#[derive(Debug)]
pub struct PackedVolume<C> {
    /// A set of spheres generated by a call to [pack_spheres](fn.pack_spheres.html).
    pub spheres: Vec<Sphere>,
    /// The container in which spheres have been packed.
    pub container: C,
}

impl<C: Container> PackedVolume<C> {
    /// Creates a new `PackedVolume` by calling [pack_spheres](fn.pack_spheres.html) with a given distribution of sphere sizes
    /// and a `container` to pack into.
    pub fn new<R: IndependentSample<f64>>(
        container: C,
        mut size_distribution: &mut R,
    ) -> PackedVolume<C> {
        PackedVolume::<C> {
            spheres: pack_spheres::<C, R>(&container, &mut size_distribution),
            container: container,
        }
    }

    /// Creates a `PackedVolume` from a pre calculated cluster of `spheres`. Useful for gathering statistics from
    /// packings generated elsewhere for comparison to the current algorithm. Also used for deserialization.
    pub fn from_vec(spheres: Vec<Sphere>, container: C) -> PackedVolume<C> {
        PackedVolume::<C> {
            spheres: spheres,
            container: container,
        }
    }

    /// Calculates the volume fraction ν = Vs/V: the volume of all spheres packed into a container
    /// divided by the volume of said container.
    ///
    /// The higest possible volume fraction for any random radii distribution is currently unknown to
    /// mathematicians. The algorithm implemented herin obtains 59.29% in a cube with side lengths of
    /// 90 with sphere radii between 0.01 and 0.02, compared with a long standing and well known
    /// [geometric compression algorithm](10.1016/j.powtec.2005.04.055) which achieved 52.89%.
    pub fn volume_fraction(&self) -> f32 {
        let vol_spheres: f32 = self.spheres.iter().map(|sphere| sphere.volume()).sum();
        vol_spheres / self.container.volume()
    }

    /// Calculates the void ratio e = Vv/Vs: the volume of all void space divided by the volume of
    /// solids in the container. Here we take 'solids' to mean volumes of all packed spheres.
    pub fn void_ratio(&self) -> f32 {
        let vol_spheres: f32 = self.spheres.iter().map(|sphere| sphere.volume()).sum();
        let vol_total = self.container.volume();
        (vol_total - vol_spheres) / vol_spheres
    }

    /// The coordination number indicates the connectivity of the packing.
    /// For any given sphere in the packing, its coordination number is defined as
    /// the number of spheres it is in contact with. This function returns the
    /// arethmetic mean of all coordination numbers in the packing, yielding a
    /// overall coordination number of the system.
    pub fn coordination_number(&self) -> f32 {
        let num_particles = self.spheres.len() as f32;
        let mut coordinations = 0;
        for idx in 0..self.spheres.len() {
            coordinations += self.sphere_contacts_count(idx);
        }
        coordinations as f32 / num_particles
    }

    /// Generates the fabric tensor of the packing. The sum of all eigenvalues phi_i,j will always equal 1.
    /// Perfectly isotropic packing should see the diagonals of this matrix = 1/3. Deviations from this value
    /// indicates the amount of anisotropy in the system.
    pub fn fabric_tensor(&self) -> Matrix3<f32> {
        let phi = |i: usize, j: usize| {
            let mut sum_all = 0.;
            for idx in 0..self.spheres.len() {
                let center = self.spheres[idx].center.coords;
                // The set of all spheres in contact with the current sphere
                let p_c = self.sphere_contacts(idx);
                // Number of spheres in contact with the current sphere
                let m_p = p_c.len() as f32;
                let mut sum_vec = 0.;
                for c in p_c.iter() {
                    let vec_n_pc = Matrix::cross(&center, &c.center.coords);
                    // The unit vector pointing from the center of the current sphere to
                    // the center of a connecting sphere
                    let n_pc = vec_n_pc / nalgebra::norm(&vec_n_pc);
                    sum_vec += n_pc[i] * n_pc[j];
                }
                sum_all += sum_vec / m_p;
            }
            // phiᵢⱼ
            1. / self.spheres.len() as f32 * sum_all
        };
        Matrix3::from_fn(|r, c| phi(r, c))
    }

    /// Returns a set of spheres connected to the sphere at a chosen index.
    fn sphere_contacts(&self, sphere_idx: usize) -> Vec<Sphere> {
        let center = self.spheres[sphere_idx].center;
        let radius = self.spheres[sphere_idx].radius;
        self.spheres
            .iter()
            .cloned()
            .filter(|sphere| {
                (nalgebra::distance(&center, &sphere.center) - (radius + sphere.radius)).abs() <
                    0.001
            })
            .collect()
    }

    /// Calculates the number of contacts a sphere has with the rest of the packed set.
    fn sphere_contacts_count(&self, sphere_idx: usize) -> usize {
        let center = self.spheres[sphere_idx].center;
        let radius = self.spheres[sphere_idx].radius;
        self.spheres
            .iter()
            .filter(|sphere| {
                (nalgebra::distance(&center, &sphere.center) - (radius + sphere.radius)).abs() <
                    0.001
            })
            .count()
    }
}

/// Packs all habitat spheres to be as dense as possible.
/// Requires a `containter` and a distribution of radii sizes.
///
/// Generally, a uniform distribution is chosen, although this library
/// accepts any distribution implementing `rand`s `IndependentSample` trait.
/// This [example](https://github.com/Libbum/spherical-cow/blob/master/examples/count_sphere_normal.rs)
/// uses a normally distributed radii range. Note that the packing is sub optimal in this case, and
/// attention must be paid when using such distributions that radii values do not become negagive.
pub fn pack_spheres<C: Container, R: IndependentSample<f64>>(
    container: &C,
    size_distribution: &mut R,
) -> Vec<Sphere> {
    // IndependentSample is already derrived for all distributions in `rand` with f64,
    // so we just downsample here instead of implementing traits on f32 for everything.
    let mut rng = rand::thread_rng();

    // Radii of three initial spheres, taken from the input distribution
    let init_radii: [f32; 3] = [
        size_distribution.ind_sample(&mut rng) as f32,
        size_distribution.ind_sample(&mut rng) as f32,
        size_distribution.ind_sample(&mut rng) as f32,
    ];

    // S := {s₁, s₂, s₃}
    let mut spheres = init_spheres(&init_radii, container);

    // F := {s₁, s₂, s₃}
    let mut front = spheres.clone();

    // Radius of new sphere to be added to the current front, taken from the input distribution
    let mut new_radius = size_distribution.ind_sample(&mut rng) as f32;

    'outer: while !front.is_empty() {
        // s₀ := s(c₀, r₀) picked at random from F
        let curr_sphere = rng.choose(&front).unwrap().clone();
        // V := {s(c', r') ∈ S : d(c₀, c') ≤ r₀ + r' + 2r}
        let set_v = spheres
            .iter()
            .cloned()
            .filter(|s_dash| {
                s_dash != &curr_sphere &&
                    nalgebra::distance(&curr_sphere.center, &s_dash.center) <=
                        curr_sphere.radius + s_dash.radius + 2. * new_radius
            })
            .collect::<Vec<_>>();

        for (s_i, s_j) in pairs(&set_v) {
            let mut set_f = identify_f(&curr_sphere, s_i, s_j, container, &set_v, new_radius);
            if !set_f.is_empty() {
                // Found at least one position to place the sphere,
                // choose one and move on
                let s_new = rng.choose(&set_f).unwrap();
                front.push(s_new.clone());
                spheres.push(s_new.clone());
                new_radius = size_distribution.ind_sample(&mut rng) as f32;
                continue 'outer;
            }
        }
        // NOTE: his is a nightly function only
        front.remove_item(&curr_sphere);
    }
    spheres
}

/// Creates three initial spheres that are tangent pairwise. The incenter of the triangle formed
/// by verticies located at the centers of each sphere is aligned at the origin.
fn init_spheres<C: Container>(radii: &[f32; 3], container: &C) -> Vec<Sphere> {
    let mut init = Vec::new();

    //            C (x,y)
    //            ^
    //           / \
    //        b /   \ a
    //         /     \
    //        /       \
    // A (0,0)--------- B (c,0)
    //            c

    // Sphere A can sit at the origin, sphere B extends outward along the x axis
    // sphere C extends outward along the y axis and complete the triangle
    let radius_a = radii[0];
    let radius_b = radii[1];
    let radius_c = radii[2];
    let distance_c = radius_a + radius_b;
    let distance_b = radius_a + radius_c;
    let distance_a = radius_c + radius_b;

    let x = (distance_b.powi(2) + distance_c.powi(2) - distance_a.powi(2)) / (2. * distance_c);
    let y = (distance_b.powi(2) - x.powi(2)).sqrt();

    // Find incenter
    let perimeter = distance_a + distance_b + distance_c;
    let incenter_x = (distance_b * distance_c + distance_c * x) / perimeter;
    let incenter_y = (distance_c * y) / perimeter;

    // Create spheres at positions shown in the diagram above, but offset such
    // that the incenter is now the origin. This offset attempts to minimise
    // bounding box issues in the sense that c may be close to or over the
    // bb boundary already
    init.push(Sphere::new(
        Point3::new(-incenter_x, -incenter_y, 0.),
        radius_a,
    ));
    init.push(Sphere::new(
        Point3::new(distance_c - incenter_x, -incenter_y, 0.),
        radius_b,
    ));
    init.push(Sphere::new(
        Point3::new(x - incenter_x, y - incenter_y, 0.),
        radius_c,
    ));

    //TODO: error, not assert
    assert!(init.iter().all(|sphere| container.contains(&sphere)));
    init
}

/// $f$ is as a set of spheres (or the empty set) such that they have a known `radius`,
/// are in outer contact with `s_1`, `s_2` and `s_3` simultaneously, are completely
/// contained in `container` and do not overlap with any element of `set_v`.
/// The set f has at most two elements, because there exist at most two spheres with
/// `radius` in outer contact with `s_1`, `s_2` and `s_3` simultaneously.
fn identify_f<C: Container>(
    s_1: &Sphere,
    s_2: &Sphere,
    s_3: &Sphere,
    container: &C,
    set_v: &Vec<Sphere>,
    radius: f32,
) -> Vec<Sphere> {

    //The center points of s_1, s_2, s_3 are verticies of a tetrahedron,
    //and the distances d_1, d_2, d_3 can be defined as the distances from these points to
    //a fourth vertex s_4, whose coordinates x,y,z must be found. This satisfies the equations
    // (x-x_1)^2+(y-y_1)^2+(z-z_1)^2=d_1^2 (1)
    // (x-x_2)^2+(y-y_2)^2+(z-z_2)^2=d_2^2 (2)
    // (x-x_3)^2+(y-y_3)^2+(z-z_3)^2=d_3^2 (3)

    //To solve this system, we subtract (1) from (2) & (3), to obtain the (linear) equations of two planes.
    //Coupling these planes to (1) we yield a quadratic system which takes the form
    // \vec u\cdot\vec r=a
    // \vec v\cdot\vec r=b
    // \vec r\cdot\vec r+\vec w\cdot\vec r=c

    // With a little bit of magic following https://axiomatic.neophilus.net/posts/2018-01-16-clustering-tangent-spheres.html
    // we can solve this system to identify r in the form
    // \vec r=\alpha\vec u+\beta\vec v+\gamma\vec t
    // Where \gamma has a quadratic solution identifying our two solutions.

    let distance_14 = s_1.radius + radius;
    let distance_24 = s_2.radius + radius;
    let distance_34 = s_3.radius + radius;

    let vector_u = s_1.center - s_2.center;
    let unitvector_u = vector_u / nalgebra::norm(&vector_u);
    let vector_v = s_1.center - s_3.center;
    let unitvector_v = vector_v / nalgebra::norm(&vector_v);
    let cross_uv = Matrix::cross(&vector_u, &vector_v);
    let unitvector_t = cross_uv / nalgebra::norm(&cross_uv);
    let vector_w = -2. * s_1.center.coords;

    let distance_a = (distance_24.powi(2) - distance_14.powi(2) + s_1.center.x.powi(2) +
                          s_1.center.y.powi(2) + s_1.center.z.powi(2) -
                          s_2.center.x.powi(2) -
                          s_2.center.y.powi(2) - s_2.center.z.powi(2)) /
        (2. * nalgebra::norm(&vector_u));
    let distance_b = (distance_34.powi(2) - distance_14.powi(2) + s_1.center.x.powi(2) +
                          s_1.center.y.powi(2) + s_1.center.z.powi(2) -
                          s_3.center.x.powi(2) -
                          s_3.center.y.powi(2) - s_3.center.z.powi(2)) /
        (2. * nalgebra::norm(&vector_v));
    let distance_c = distance_14.powi(2) - s_1.center.x.powi(2) - s_1.center.y.powi(2) -
        s_1.center.z.powi(2);

    let dot_uv = nalgebra::dot(&unitvector_u, &unitvector_v);
    let dot_wt = nalgebra::dot(&vector_w, &unitvector_t);
    let dot_uw = nalgebra::dot(&unitvector_u, &vector_w);
    let dot_vw = nalgebra::dot(&unitvector_v, &vector_w);

    let alpha = (distance_a - distance_b * dot_uv) / (1. - dot_uv.powi(2));
    let beta = (distance_b - distance_a * dot_uv) / (1. - dot_uv.powi(2));
    let value_d = alpha.powi(2) + beta.powi(2) + 2. * alpha * beta * dot_uv + alpha * dot_uw +
        beta * dot_vw - distance_c;
    let dot_wt_2 = dot_wt.powi(2);
    let value_4d = 4. * value_d;

    let mut f = Vec::new();
    // There is a possiblity of obtaining imaginary solutions in gamma,
    // so we must check this comparison. TODO: Would be nice to have
    // some quick way of verifying this configuration and deny it early.
    if dot_wt_2 > value_4d {
        let gamma_pos = 0.5 * (-dot_wt + (dot_wt.powi(2) - 4. * value_d).sqrt());
        let gamma_neg = 0.5 * (-dot_wt - (dot_wt.powi(2) - 4. * value_d).sqrt());

        let s_4_positive = Sphere::new(
            Point3::from_coordinates(
                alpha * unitvector_u + beta * unitvector_v + gamma_pos * unitvector_t,
            ),
            radius,
        );
        let s_4_negative = Sphere::new(
            Point3::from_coordinates(
                alpha * unitvector_u + beta * unitvector_v + gamma_neg * unitvector_t,
            ),
            radius,
        );

        // Make sure the spheres are bounded by the containing geometry and do not overlap any spheres in V
        if container.contains(&s_4_positive) && !set_v.iter().any(|v| v.overlaps(&s_4_positive)) {
            f.push(s_4_positive);
        }
        if container.contains(&s_4_negative) && !set_v.iter().any(|v| v.overlaps(&s_4_negative)) {
            f.push(s_4_negative);
        }
    }
    f
}

/// Calculates all possible pairs of a `set` of values.
fn pairs(set: &[Sphere]) -> Vec<(&Sphere, &Sphere)> {
    let n = set.len();

    if n == 2 {
        let mut minimal = Vec::new();
        minimal.push((&set[0], &set[1]));
        minimal
    } else {
        let mut vec_pairs = Vec::new();
        if n > 2 {
            // 0..n - m, but m = 2 and rust is non inclusive with its for loops
            for k in 0..n - 1 {
                let subset = &set[k + 1..n];
                vec_pairs.append(&mut subset
                    .iter()
                    .zip(repeat(&set[k]).take(subset.len()))
                    .collect::<Vec<_>>());
            }
        }
        vec_pairs
    }
}
