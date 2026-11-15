//! Where each shadow map goes in the depth array: a square of a power-of-two
//! side in some layer. Laid out largest first by halving free squares (a
//! buddy allocator, as Godot's shadow atlas quadrants): with power-of-two
//! sides placed in decreasing order, every layer fills without holes before
//! the next one starts.

/// A map's place: its layer and its square (texture coordinates: x0, y0,
/// x1, y1).
pub(crate) type Place = (usize, [f32; 4]);

/// Places squares of the given sides (powers of two, at most `layer`) in
/// layers of side `layer`. Returns each one's place, in the order given,
/// and how many layers they take. The same sides in the same order always
/// land in the same places (the shadow cache relies on it).
pub(crate) fn lay_out(sides: &[u32], layer: u32) -> (Vec<Place>, usize) {
    let layer = layer.max(1);
    let mut order: Vec<usize> = (0..sides.len()).collect();
    // Largest first; ties keep their order (the sort is stable).
    order.sort_by_key(|&i| std::cmp::Reverse(sides[i].clamp(1, layer)));
    // Free squares: (layer, x, y, side) in texels.
    let mut free: Vec<(usize, u32, u32, u32)> = Vec::new();
    let mut layers = 0;
    let mut places = vec![(0, [0.0; 4]); sides.len()];
    for i in order {
        let side = sides[i].clamp(1, layer).next_power_of_two().min(layer);
        // The smallest free square that holds it (the first of those, so the
        // layout fills in order), else a new layer.
        let found = free
            .iter()
            .enumerate()
            .filter(|(_, f)| f.3 >= side)
            .min_by_key(|(k, f)| (f.3, *k))
            .map(|(k, _)| k);
        let (l, x, y, mut s) = match found {
            Some(k) => free.remove(k),
            None => {
                layers += 1;
                (layers - 1, 0, 0, layer)
            }
        };
        // Halved down to its side; the other three quarters stay free (in
        // reading order, so the next of the same side takes the next one).
        while s > side {
            s /= 2;
            let rest = [(l, x + s, y, s), (l, x, y + s, s), (l, x + s, y + s, s)];
            free.splice(0..0, rest);
        }
        let f = layer as f32;
        places[i] = (
            l,
            [
                x as f32 / f,
                y as f32 / f,
                (x + side) as f32 / f,
                (y + side) as f32 / f,
            ],
        );
    }
    (places, layers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_layers_and_quarters() {
        // A whole map, then a point light's six faces (a quarter each).
        let (p, layers) = lay_out(&[2048, 1024, 1024, 1024, 1024, 1024, 1024], 2048);
        assert_eq!(layers, 3);
        assert_eq!(p[0], (0, [0.0, 0.0, 1.0, 1.0]));
        assert_eq!(p[1], (1, [0.0, 0.0, 0.5, 0.5]));
        assert_eq!(p[2], (1, [0.5, 0.0, 1.0, 0.5]));
        assert_eq!(p[4], (1, [0.5, 0.5, 1.0, 1.0]));
        assert_eq!(p[5], (2, [0.0, 0.0, 0.5, 0.5]));
    }

    #[test]
    fn small_maps_fill_the_gaps_of_larger_ones() {
        // Given out of order: placed largest first, the small ones in what
        // the large ones left.
        let sides = [256, 1024, 512, 1024, 256, 512, 1024];
        let (p, layers) = lay_out(&sides, 2048);
        // 3 × 1024² + 2 × 512² + 2 × 256² fit one 2048² layer.
        assert_eq!(layers, 1);
        // No two overlap.
        for a in 0..p.len() {
            for b in a + 1..p.len() {
                let (ra, rb) = (p[a].1, p[b].1);
                let apart = ra[2] <= rb[0] || rb[2] <= ra[0] || ra[3] <= rb[1] || rb[3] <= ra[1];
                assert!(apart, "{a} {ra:?} and {b} {rb:?} overlap");
            }
        }
        // Each the size asked.
        for (i, &s) in sides.iter().enumerate() {
            let r = p[i].1;
            assert_eq!(((r[2] - r[0]) * 2048.0) as u32, s);
        }
    }

    #[test]
    fn the_same_sides_land_in_the_same_places() {
        let sides = [512, 2048, 512, 1024];
        assert_eq!(lay_out(&sides, 2048), lay_out(&sides, 2048));
    }
}
