//! Rank queries on plain bitvectors.
//!
//! The structure is the same as `rank_support_v` in [SDSL](https://github.com/simongog/sdsl-lite):
//!
//! > Gog, Petri: Optimized succinct data structures for massive data.
//! > Software: Practice and Experience, 2014.
//! > [DOI: 10.1002/spe.2198](https://doi.org/10.1002/spe.2198)
//!
//! The original version is called rank9:
//!
//! > Vigna: Broadword Implementation of Rank/Select Queries. WEA 2008.
//! > [DOI: 10.1007/978-3-540-68552-4_12](https://doi.org/10.1007/978-3-540-68552-4_12)
//!
//! We divide the bitvector into blocks of 2^9 = 512 bits.
//! Each block is further divided into 8 words of 64 bits each.
//! For each block, we store two 64-bit integers:
//!
//! * The first stores the number of ones in previous blocks.
//! * The second stores, for each word except the first, the number of ones in previous words using 9 bits per word.
//!
//! The space overhead is 25%.

use crate::bit_vector::BitVector;
use crate::ops::BitVec;
use crate::raw_vector::GetRaw;
use crate::serialize::Serialize;
use crate::bits;

use std::{cmp, io};

//-----------------------------------------------------------------------------

/// Rank support structure for plain bitvectors.
///
/// The structure depends on the parent bitvector and assumes that the parent remains unchanged.
/// Using the [`BitVector`] interface is usually more convenient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankSupport {
    // No RawVector or bits::read_int because we want to avoid branching.
    samples: Vec<u64>,
}

impl RankSupport {
    const BLOCK_SIZE: usize = 512;
    const RELATIVE_RANK_BITS: usize = 9;
    const RELATIVE_RANK_MASK: usize = 0x1FF;
    const WORDS_PER_BLOCK: usize = 8;
    const WORD_MASK: usize = 0x7;

    /// Returns the number of 512-bit blocks in the bitvector.
    pub fn blocks(&self) -> usize {
        self.samples.len() / 2
    }

    /// Builds a rank support structure for the parent bitvector.
    ///
    /// # Examples
    ///
    /// ```
    /// use simple_sds::bit_vector::BitVector;
    /// use simple_sds::bit_vector::rank_support::RankSupport;
    ///
    /// let data = vec![false, true, true, false, true, false, true, true, false, false, false];
    /// let bv: BitVector = data.into_iter().collect();
    /// let rs = RankSupport::new(&bv);
    /// assert_eq!(rs.blocks(), 1);
    /// ```
    pub fn new(parent: &BitVector) -> RankSupport {
        let words = bits::bits_to_words(parent.len());
        let blocks = (parent.len() + Self::BLOCK_SIZE - 1) / Self::BLOCK_SIZE;
        let mut samples: Vec<u64> = Vec::with_capacity(2 * blocks);

        let mut ones: usize = 0;
        for block in 0..blocks {
            samples.push(ones as u64);
            let mut block_ones: usize = 0;
            let mut relative_ranks: u64 = 0;
            let block_words = cmp::min(Self::WORDS_PER_BLOCK, words - block * Self::WORDS_PER_BLOCK);
            for word in 0..block_words {
                block_ones += parent.data.word(block * Self::WORDS_PER_BLOCK + word).count_ones() as usize;
                relative_ranks |= (block_ones << (word * Self::RELATIVE_RANK_BITS)) as u64;
            }
            // Clear the high bit. We don't store the relative rank after all 8 words.
            relative_ranks &= bits::low_set((Self::WORDS_PER_BLOCK - 1) * Self::RELATIVE_RANK_BITS) as u64;
            samples.push(relative_ranks);
            ones += block_ones;
        }

        RankSupport {
            samples: samples,
        }
    }

    /// Returns the rank at the specified index of the bitvector.
    ///
    /// # Arguments
    ///
    /// * `parent`: The parent bitvector.
    /// * `index`: Index in the binary array or value in the integer array.
    ///
    /// # Examples
    ///
    /// ```
    /// use simple_sds::bit_vector::BitVector;
    /// use simple_sds::bit_vector::rank_support::RankSupport;
    ///
    /// let data = vec![false, true, true, false, true, false, true, true, false, false, false];
    /// let bv: BitVector = data.into_iter().collect();
    /// let rs = RankSupport::new(&bv);
    /// assert_eq!(rs.rank(&bv, 0), 0);
    /// assert_eq!(rs.rank(&bv, 1), 0);
    /// assert_eq!(rs.rank(&bv, 2), 1);
    /// assert_eq!(rs.rank(&bv, 7), 4);
    /// ```
    ///
    /// # Panics
    ///
    /// May panic if `index >= parent.len()`.
    pub fn rank(&self, parent: &BitVector, index: usize) -> usize {
        let block = index / Self::BLOCK_SIZE;
        let (word, offset) = bits::split_offset(index);

        // Rank at the start of the block.
        let block_start = self.samples[2 * block] as usize;

        // Transform the absolute word index into a relative word index within the block.
        // Then reorder the words 0..8 to 1..8, 0, because the second sample stores relative
        // ranks for words 1..8.
        let relative = ((word & Self::WORD_MASK) + Self::WORDS_PER_BLOCK - 1) & Self::WORD_MASK;

        // Relative rank at the start of the word.
        let word_start = (self.samples[2 * block + 1] >> (relative * Self::RELATIVE_RANK_BITS)) as usize & Self::RELATIVE_RANK_MASK;

        // Relative rank within the word.
        let within_word = (parent.data.word(word) & bits::low_set(offset)).count_ones() as usize;

        block_start + word_start + within_word
    }
}

//-----------------------------------------------------------------------------

impl Serialize for RankSupport {
    fn serialize_header<T: io::Write>(&self, writer: &mut T) -> io::Result<()> {
        self.samples.serialize_header(writer)?;
        Ok(())
    }

    fn serialize_body<T: io::Write>(&self, writer: &mut T) -> io::Result<()> {
        self.samples.serialize_body(writer)?;
        Ok(())
    }

    fn load<T: io::Read>(reader: &mut T) -> io::Result<Self> {
        let samples = Vec::<u64>::load(reader)?;
        Ok(RankSupport {
            samples: samples,
        })
    }

    fn size_in_bytes(&self) -> usize {
        self.samples.size_in_bytes()
    }
}

//-----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bit_vector::BitVector;
    use crate::ops::BitVec;
    use crate::raw_vector::{RawVector, GetRaw, PushRaw};
    use crate::serialize;
    use std::fs;
    use rand::Rng;

    #[test]
    fn empty_vector() {
        let bv = BitVector::from(RawVector::new());
        let rs = RankSupport::new(&bv);
        assert_eq!(rs.blocks(), 0, "Non-zero rank blocks for empty vector");
    }

    fn raw_vector(len: usize) -> RawVector {
        let mut data = RawVector::with_capacity(len);
        let mut rng = rand::thread_rng();
        while data.len() < len {
            let value: u64 =  rng.gen();
            let bits = cmp::min(bits::WORD_BITS, len - data.len());
            data.push_int(value, bits);
        }
        assert_eq!(data.len(), len, "Invalid length for random RawVector");
        data
    }

    fn test_vector(len: usize, blocks: usize) {
        let data = raw_vector(len);
        let bv = BitVector::from(data.clone());
        let rs = RankSupport::new(&bv);
        assert_eq!(bv.len(), len, "Invalid bitvector length at {}", len);
        assert_eq!(rs.blocks(), blocks, "Invalid number of rank blocks at {}", len);
        let mut count: usize = 0;
        for i in 0..bv.len() {
            assert_eq!(rs.rank(&bv, i), count, "Invalid rank({}) at {}", i, len);
            count += data.bit(i) as usize;
        }
    }

    #[test]
    fn non_empty_vector() {
        test_vector(4095, 8);
        test_vector(4096, 8);
        test_vector(4097, 9);
    }

    #[test]
    fn serialize_rank_support() {
        let data = raw_vector(5187);
        let bv = BitVector::from(data);
        let original = RankSupport::new(&bv);

        let filename = serialize::temp_file_name("rank-support");
        serialize::serialize_to(&original, &filename).unwrap();

        let copy: RankSupport = serialize::load_from(&filename).unwrap();
        assert_eq!(copy, original, "Serialization changed the RankSupport");

        fs::remove_file(&filename).unwrap();
    }
}

//-----------------------------------------------------------------------------
