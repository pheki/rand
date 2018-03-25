// Copyright 2017-2018 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// https://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Thread-local random number generator

use std::cell::UnsafeCell;
use std::rc::Rc;

use {RngCore, CryptoRng, SeedableRng, EntropyRng};
use prng::hc128::Hc128Core;
use {Distribution, Uniform, Rng, Error};
use reseeding::ReseedingRng;

// Rationale for using `UnsafeCell` in `ThreadRng`:
//
// Previously we used a `RefCell`, with an overhead of ~15%. There will only
// ever be one mutable reference to the interior of the `UnsafeCell`, because
// we only have such a reference inside `next_u32`, `next_u64`, etc. Within a
// single thread (which is the definition of `ThreadRng`), there will only ever
// be one of these methods active at a time.
//
// A possible scenario where there could be multiple mutable references is if
// `ThreadRng` is used inside `next_u32` and co. But the implementation is
// completely under our control. We just have to ensure none of them use
// `ThreadRng` internally, which is nonsensical anyway. We should also never run
// `ThreadRng` in destructors of its implementation, which is also nonsensical.
//
// The additional `Rc` is not strictly neccesary, and could be removed. For now
// it ensures `ThreadRng` stays `!Send` and `!Sync`, and implements `Clone`.


// Number of generated bytes after which to reseed `TreadRng`.
//
// The time it takes to reseed HC-128 is roughly equivalent to generating 7 KiB.
// We pick a treshold here that is large enough to not reduce the average
// performance too much, but also small enough to not make reseeding something
// that basically never happens.
const THREAD_RNG_RESEED_THRESHOLD: u64 = 32*1024*1024; // 32 MiB

/// The type returned by [`thread_rng`], essentially just a reference to the
/// PRNG in thread-local memory. Cloning this handle just produces a new
/// reference to the same thread-local generator.
/// 
/// [`thread_rng`]: fn.thread_rng.html
#[derive(Clone, Debug)]
pub struct ThreadRng {
    rng: Rc<UnsafeCell<ReseedingRng<Hc128Core, EntropyRng>>>,
}

thread_local!(
    static THREAD_RNG_KEY: Rc<UnsafeCell<ReseedingRng<Hc128Core, EntropyRng>>> = {
        let mut entropy_source = EntropyRng::new();
        let r = Hc128Core::from_rng(&mut entropy_source).unwrap_or_else(|err|
                panic!("could not initialize thread_rng: {}", err));
        let rng = ReseedingRng::new(r,
                                    THREAD_RNG_RESEED_THRESHOLD,
                                    entropy_source);
        Rc::new(UnsafeCell::new(rng))
    }
);

/// Retrieve the lazily-initialized thread-local random number
/// generator, seeded by the system. Intended to be used in method
/// chaining style, e.g. `thread_rng().gen::<i32>()`, or cached locally, e.g.
/// `let mut rng = thread_rng();`.
///
/// `ThreadRng` uses [`ReseedingRng`] wrapping the same PRNG as [`StdRng`],
/// which is reseeded after generating 32 MiB of random data. A single instance
/// is cached per thread and the returned `ThreadRng` is a reference to this
/// instance — hence `ThreadRng` is neither `Send` nor `Sync` but is safe to use
/// within a single thread. This RNG is seeded and reseeded via [`EntropyRng`]
/// as required.
/// 
/// Note that the reseeding is done as an extra precaution against entropy
/// leaks and is in theory unnecessary — to predict `thread_rng`'s output, an
/// attacker would have to either determine most of the RNG's seed or internal
/// state, or crack the algorithm used.
/// 
/// Like [`StdRng`], `ThreadRng` is a cryptographically secure PRNG. The current
/// algorithm used is [HC-128], which is an array-based PRNG that trades memory
/// usage for better performance. This makes it similar to ISAAC, the algorithm
/// used in `ThreadRng` before rand 0.5.
///
/// [`ReseedingRng`]: reseeding/struct.ReseedingRng.html
/// [`StdRng`]: struct.StdRng.html
/// [`EntropyRng`]: struct.EntropyRng.html
/// [HC-128]: struct.Hc128Rng.html
pub fn thread_rng() -> ThreadRng {
    ThreadRng { rng: THREAD_RNG_KEY.with(|t| t.clone()) }
}

impl RngCore for ThreadRng {
    #[inline(always)]
    fn next_u32(&mut self) -> u32 {
        unsafe { (*self.rng.get()).next_u32() }
    }

    #[inline(always)]
    fn next_u64(&mut self) -> u64 {
        unsafe { (*self.rng.get()).next_u64() }
    }

    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        unsafe { (*self.rng.get()).fill_bytes(bytes) }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), Error> {
        unsafe { (*self.rng.get()).try_fill_bytes(dest) }
    }
}

impl CryptoRng for ThreadRng {}

/// DEPRECATED: use `thread_rng().gen()` instead.
///
/// Generates a random value using the thread-local random number generator.
///
/// This is simply a shortcut for `thread_rng().gen()`. See [`thread_rng`] for
/// documentation of the entropy source and [`Rand`] for documentation of
/// distributions and type-specific generation.
///
/// # Examples
///
/// ```
/// let x = rand::random::<u8>();
/// println!("{}", x);
///
/// let y = rand::random::<f64>();
/// println!("{}", y);
///
/// if rand::random() { // generates a boolean
///     println!("Better lucky than good!");
/// }
/// ```
///
/// If you're calling `random()` in a loop, caching the generator as in the
/// following example can increase performance.
///
/// ```
/// use rand::Rng;
///
/// let mut v = vec![1, 2, 3];
///
/// for x in v.iter_mut() {
///     *x = rand::random()
/// }
///
/// // can be made faster by caching thread_rng
///
/// let mut rng = rand::thread_rng();
///
/// for x in v.iter_mut() {
///     *x = rng.gen();
/// }
/// ```
///
/// [`thread_rng`]: fn.thread_rng.html
/// [`Rand`]: trait.Rand.html
#[deprecated(since="0.5.0", note="removed in favor of thread_rng().gen()")]
#[inline]
pub fn random<T>() -> T where Uniform: Distribution<T> {
    thread_rng().gen()
}

#[cfg(test)]
mod test {
    #[test]
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "emscripten"))))]
    fn test_thread_rng() {
        use Rng;
        let mut r = ::thread_rng();
        r.gen::<i32>();
        let mut v = [1, 1, 1];
        r.shuffle(&mut v);
        let b: &[_] = &[1, 1, 1];
        assert_eq!(v, b);
        assert_eq!(r.gen_range(0, 1), 0);
    }

    #[test]
    #[allow(deprecated)]
    fn test_random() {
        use super::random;
        // not sure how to test this aside from just getting some values
        let _n : usize = random();
        let _f : f32 = random();
        let _o : Option<Option<i8>> = random();
        let _many : ((),
                     (usize,
                      isize,
                      Option<(u32, (bool,))>),
                     (u8, i8, u16, i16, u32, i32, u64, i64),
                     (f32, (f64, (f64,)))) = random();
    }
}
