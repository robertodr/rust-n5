//! Interfaces for the [N5 "Not HDF5" n-dimensional tensor file system storage
//! format](https://github.com/saalfeldlab/n5) created by the Saalfeld lab at
//! Janelia Research Campus.


// TODO: this does not run the test for recent stable rust because `test`
// is no longer set during doc tests. When 1.40 stabilizes and is the MSRV
// this can be changed from `test` to `doctest` and will work correctly.
#[cfg(all(test, feature = "filesystem"))]
doc_comment::doctest!("../README.md");


#[macro_use]
pub extern crate smallvec;


use std::io::{
    Error,
    ErrorKind,
};
use std::marker::PhantomData;
use std::time::SystemTime;

use byteorder::{
    BigEndian,
    ByteOrder,
    ReadBytesExt,
    WriteBytesExt,
};
use serde::{
    Deserialize,
    Serialize,
};
use smallvec::SmallVec;

use crate::compression::Compression;

pub mod compression;
#[macro_use]
pub mod data_type;
pub use data_type::*;
#[cfg(feature = "filesystem")]
pub mod filesystem;
#[cfg(feature = "use_ndarray")]
pub mod ndarray;
pub mod prelude;

#[cfg(test)]
#[macro_use]
pub(crate) mod tests;

pub use semver::Version;


const COORD_SMALLVEC_SIZE: usize = 6;
pub type CoordVec<T> = SmallVec<[T; COORD_SMALLVEC_SIZE]>;
pub type BlockCoord = CoordVec<u32>;
pub type GridCoord = CoordVec<u64>;


/// Version of the Java N5 spec supported by this library.
pub const VERSION: Version = Version {
    major: 2,
    minor: 1,
    patch: 3,
    pre: Vec::new(),
    build: Vec::new(),
};

pub fn is_version_compatible(s: &Version, other: &Version) -> bool {
    other.major <= s.major
}

/// Key name for the version attribute in the container root.
pub const VERSION_ATTRIBUTE_KEY: &str = "n5";

#[derive(Clone, Debug)]
pub struct DataBlockMetadata {
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub modified: Option<SystemTime>,
    pub size: Option<u64>,
}

/// Non-mutating operations on N5 containers.
pub trait N5Reader {
    /// Get the N5 specification version of the container.
    fn get_version(&self) -> Result<Version, Error>;

    /// Get attributes for a dataset.
    fn get_dataset_attributes(&self, path_name: &str) -> Result<DatasetAttributes, Error>;

    /// Test whether a group or dataset exists.
    fn exists(&self, path_name: &str) -> Result<bool, Error>;

    /// Test whether a dataset exists.
    fn dataset_exists(&self, path_name: &str) -> Result<bool, Error> {
        Ok(self.exists(path_name)? && self.get_dataset_attributes(path_name).is_ok())
    }

    /// Get a URI string for a data block.
    ///
    /// Whether this requires that the dataset and block exist is currently
    /// implementation dependent. Whether this URI is a URL is implementation
    /// dependent.
    fn get_block_uri(&self, path_name: &str, grid_position: &[u64]) -> Result<String, Error>;

    /// Read a single dataset block into a linear vec.
    fn read_block<T>(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
        grid_position: GridCoord,
    ) -> Result<Option<VecDataBlock<T>>, Error>
        where VecDataBlock<T>: DataBlock<T> + ReadableDataBlock,
              T: ReflectedType;

    /// Read a single dataset block into an existing buffer.
    fn read_block_into<T: ReflectedType, B: DataBlock<T> + ReinitDataBlock<T> + ReadableDataBlock>(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
        grid_position: GridCoord,
        block: &mut B,
    ) -> Result<Option<()>, Error>;

    /// Read metadata about a block.
    fn block_metadata(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
        grid_position: &[u64],
    ) -> Result<Option<DataBlockMetadata>, Error>;

    /// List all attributes of a group.
    fn list_attributes(&self, path_name: &str) -> Result<serde_json::Value, Error>;
}

pub trait N5Lister : N5Reader {
    /// List all groups (including datasets) in a group.
    fn list(&self, path_name: &str) -> Result<Vec<String>, Error>;
}

/// Mutating operations on N5 containers.
pub trait N5Writer : N5Reader {
    /// Set a single attribute.
    fn set_attribute<T: Serialize>(
        &self, // TODO: should this be mut for semantics?
        path_name: &str,
        key: String,
        attribute: T,
    ) -> Result<(), Error> {
        self.set_attributes(
            path_name,
            vec![(key, serde_json::to_value(attribute)?)].into_iter().collect())
    }

    /// Set a map of attributes.
    fn set_attributes(
        &self, // TODO: should this be mut for semantics?
        path_name: &str,
        attributes: serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), Error>;

    /// Set mandatory dataset attributes.
    fn set_dataset_attributes(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
    ) -> Result<(), Error> {
        if let serde_json::Value::Object(map) = serde_json::to_value(data_attrs)? {
            self.set_attributes(path_name, map)
        } else {
            panic!("Impossible: DatasetAttributes serializes to object")
        }
    }

    /// Create a group (directory).
    fn create_group(&self, path_name: &str) -> Result<(), Error>;

    /// Create a dataset. This will create the dataset group and attributes,
    /// but not populate any block data.
    fn create_dataset(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
    ) -> Result<(), Error> {
        self.create_group(path_name)?;
        self.set_dataset_attributes(path_name, data_attrs)
    }

    /// Remove the N5 container.
    fn remove_all(&self) -> Result<(), Error> {
        self.remove("")
    }

    /// Remove a group or dataset (directory and all contained files).
    ///
    /// This will wait on locks acquired by other writers or readers.
    fn remove(
        &self,
        path_name: &str,
    ) -> Result<(), Error>;

    fn write_block<T, B: DataBlock<T> + WriteableDataBlock>(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
        block: &B,
    ) -> Result<(), Error>;

    /// Delete a block from a dataset.
    ///
    /// Returns `true` if the block does not exist on the backend at the
    /// completion of the call.
    fn delete_block(
        &self,
        path_name: &str,
        grid_position: &[u64],
    ) -> Result<bool, Error>;
}


/// Attributes of a tensor dataset.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DatasetAttributes {
    /// Dimensions of the entire dataset, in voxels.
    dimensions: GridCoord,
    /// Size of each block, in voxels.
    block_size: BlockCoord,
    /// Element data type.
    data_type: DataType,
    /// Compression scheme for voxel data in each block.
    compression: compression::CompressionType,
}

impl DatasetAttributes {
    pub fn new(
        dimensions: GridCoord,
        block_size: BlockCoord,
        data_type: DataType,
        compression: compression::CompressionType,
    ) -> DatasetAttributes {
        assert_eq!(dimensions.len(), block_size.len(),
            "Number of dataset dimensions must match number of block size dimensions.");
        DatasetAttributes {
            dimensions,
            block_size,
            data_type,
            compression,
        }
    }

    pub fn get_dimensions(&self) -> &[u64] {
        &self.dimensions
    }

    pub fn get_block_size(&self) -> &[u32] {
        &self.block_size
    }

    pub fn get_data_type(&self) -> &DataType {
        &self.data_type
    }

    pub fn get_compression(&self) -> &compression::CompressionType {
        &self.compression
    }

    pub fn get_ndim(&self) -> usize {
        self.dimensions.len()
    }

    /// Get the total number of elements possible given the dimensions.
    pub fn get_num_elements(&self) -> usize {
        self.dimensions.iter().map(|&d| d as usize).product()
    }

    /// Get the total number of elements possible in a block.
    pub fn get_block_num_elements(&self) -> usize {
        self.block_size.iter().map(|&d| d as usize).product()
    }

    /// Get the upper bound extent of grid coordinates.
    pub fn get_grid_extent(&self) -> GridCoord {
        self.dimensions.iter()
            .zip(self.block_size.iter().cloned().map(u64::from))
            .map(|(d, b)| (d + 1) / b + (if d % b != 0 {1} else {0}))
            .collect()
    }

    /// Get the total number of blocks.
    /// ```
    /// use n5::prelude::*;
    /// use n5::smallvec::smallvec;
    /// let attrs = DatasetAttributes::new(
    ///     smallvec![50, 40, 30],
    ///     smallvec![11, 10, 10],
    ///     DataType::UINT8,
    ///     n5::compression::CompressionType::default(),
    /// );
    /// assert_eq!(attrs.get_num_blocks(), 60);
    /// ```
    pub fn get_num_blocks(&self) -> u64 {
        self.get_grid_extent().iter().product()
    }

    /// Check whether a block grid position is in the bounds of this dataset.
    /// ```
    /// use n5::prelude::*;
    /// use n5::smallvec::smallvec;
    /// let attrs = DatasetAttributes::new(
    ///     smallvec![50, 40, 30],
    ///     smallvec![11, 10, 10],
    ///     DataType::UINT8,
    ///     n5::compression::CompressionType::default(),
    /// );
    /// assert!(attrs.in_bounds(&smallvec![4, 3, 2]));
    /// assert!(!attrs.in_bounds(&smallvec![5, 3, 2]));
    /// ```
    pub fn in_bounds(&self, grid_position: &GridCoord) -> bool {
        self.dimensions.len() == grid_position.len() &&
        self.get_grid_extent().iter()
            .zip(grid_position.iter())
            .all(|(&bound, &coord)| coord < bound)
    }
}


pub struct BlockHeader {
    size: BlockCoord,
    grid_position: GridCoord,
    num_el: usize,
}

pub trait ReinitDataBlock<T> {
    fn reinitialize(&mut self, header: BlockHeader);

    fn reinitialize_with<B: DataBlock<T>>(&mut self, other: &B);
}

pub trait ReadableDataBlock {
    /// Unlike Java N5, read the stream directly into the block data instead
    /// of creating a copied byte buffer.
    fn read_data<R: std::io::Read>(&mut self, source: R) -> std::io::Result<()>;
}

pub trait WriteableDataBlock {
    fn write_data<W: std::io::Write>(&self, target: W) -> std::io::Result<()>;
}

/// Common interface for data blocks of element (rust) type `T`.
///
/// To enable custom types to be written to N5 volumes, implement this trait.
pub trait DataBlock<T> {
    fn get_size(&self) -> &[u32];

    fn get_grid_position(&self) -> &[u64];

    fn get_data(&self) -> &[T];

    fn get_num_elements(&self) -> u32;

    fn get_header(&self) -> BlockHeader {
        BlockHeader {
            size: self.get_size().into(),
            grid_position: self.get_grid_position().into(),
            num_el: self.get_num_elements() as usize,
        }
    }
}

/// A generic data block container wrapping any type that can be taken as a
/// slice ref.
#[derive(Clone)]
pub struct SliceDataBlock<T: ReflectedType, C> {
    data_type: PhantomData<T>,
    size: BlockCoord,
    grid_position: GridCoord,
    data: C,
}

/// A linear vector storing a data block. All read data blocks are returned as
/// this type.
pub type VecDataBlock<T> = SliceDataBlock<T, Vec<T>>;

impl<T: ReflectedType, C> SliceDataBlock<T, C> {
    pub fn new(size: BlockCoord, grid_position: GridCoord, data: C) -> SliceDataBlock<T, C> {
        SliceDataBlock {
            data_type: PhantomData,
            size,
            grid_position,
            data,
        }
    }

    pub fn into_data(self) -> C {
        self.data
    }
}

impl<T: ReflectedType> ReinitDataBlock<T> for VecDataBlock<T> {
    fn reinitialize(&mut self, header: BlockHeader) {
        self.size = header.size;
        self.grid_position = header.grid_position;
        self.data.resize_with(header.num_el, Default::default);
    }

    fn reinitialize_with<B: DataBlock<T>>(&mut self, other: &B) {
        self.size = other.get_size().into();
        self.grid_position = other.get_grid_position().into();
        self.data.clear();
        self.data.extend_from_slice(other.get_data());
    }
}

macro_rules! vec_data_block_impl {
    ($ty_name:ty, $bo_read_fn:ident, $bo_write_fn:ident) => {
        impl<C: AsMut<[$ty_name]>> ReadableDataBlock for SliceDataBlock<$ty_name, C> {
            fn read_data<R: std::io::Read>(&mut self, mut source: R) -> std::io::Result<()> {
                source.$bo_read_fn::<BigEndian>(self.data.as_mut())
            }
        }

        impl<C: AsRef<[$ty_name]>> WriteableDataBlock for SliceDataBlock<$ty_name, C> {
            fn write_data<W: std::io::Write>(&self, mut target: W) -> std::io::Result<()> {
                const CHUNK: usize = 256;
                let mut buf: [u8; CHUNK * std::mem::size_of::<$ty_name>()] =
                    [0; CHUNK * std::mem::size_of::<$ty_name>()];

                for c in self.data.as_ref().chunks(CHUNK) {
                    let byte_len = c.len() * std::mem::size_of::<$ty_name>();
                    BigEndian::$bo_write_fn(c, &mut buf[..byte_len]);
                    target.write_all(&buf[..byte_len])?;
                }

                Ok(())
            }
        }
    }
}

vec_data_block_impl!(u16, read_u16_into, write_u16_into);
vec_data_block_impl!(u32, read_u32_into, write_u32_into);
vec_data_block_impl!(u64, read_u64_into, write_u64_into);
vec_data_block_impl!(i16, read_i16_into, write_i16_into);
vec_data_block_impl!(i32, read_i32_into, write_i32_into);
vec_data_block_impl!(i64, read_i64_into, write_i64_into);
vec_data_block_impl!(f32, read_f32_into, write_f32_into);
vec_data_block_impl!(f64, read_f64_into, write_f64_into);

impl<C: AsMut<[u8]>> ReadableDataBlock for SliceDataBlock<u8, C> {
    fn read_data<R: std::io::Read>(&mut self, mut source: R) -> std::io::Result<()> {
        source.read_exact(self.data.as_mut())
    }
}

impl<C: AsRef<[u8]>> WriteableDataBlock for SliceDataBlock<u8, C> {
    fn write_data<W: std::io::Write>(&self, mut target: W) -> std::io::Result<()> {
        target.write_all(self.data.as_ref())
    }
}

impl<C: AsMut<[i8]>> ReadableDataBlock for SliceDataBlock<i8, C> {
    fn read_data<R: std::io::Read>(&mut self, mut source: R) -> std::io::Result<()> {
        // Unsafe necessary here because we need a &mut [u8] to avoid doing
        // individual reads to the i8 data. This is safe.
        // Note that byteorder's read_i8_into is not used, because it is also
        // unsafe under the hood and moreso than this incantation.
        let data_ref = unsafe { &mut *(self.data.as_mut() as *mut [i8] as *mut [u8]) };
        source.read_exact(data_ref)
    }
}

impl<C: AsRef<[i8]>> WriteableDataBlock for SliceDataBlock<i8, C> {
    fn write_data<W: std::io::Write>(&self, mut target: W) -> std::io::Result<()> {
        // Unsafe necessary here because we need a &mut [u8] to avoid doing
        // individual writes from the i8 data. This is safe.
        let data_ref = unsafe { &*(self.data.as_ref() as *const [i8] as *const [u8]) };
        target.write_all(data_ref)
    }
}

impl<T: ReflectedType, C: AsRef<[T]>> DataBlock<T> for SliceDataBlock<T, C> {
    fn get_size(&self) -> &[u32] {
        &self.size
    }

    fn get_grid_position(&self) -> &[u64] {
        &self.grid_position
    }

    fn get_data(&self) -> &[T] {
        self.data.as_ref()
    }

    fn get_num_elements(&self) -> u32 {
        self.data.as_ref().len() as u32
    }
}


pub trait DefaultBlockHeaderReader<R: std::io::Read> {
    fn read_block_header(
        buffer: &mut R,
        grid_position: GridCoord,
    ) -> std::io::Result<BlockHeader> {

        let mode = buffer.read_u16::<BigEndian>()?;
        let ndim = buffer.read_u16::<BigEndian>()?;
        let mut size = smallvec![0; ndim as usize];
        buffer.read_u32_into::<BigEndian>(&mut size)?;
        let num_el = match mode {
            0 => size.iter().product(),
            1 => buffer.read_u32::<BigEndian>()?,
            _ => return Err(Error::new(ErrorKind::InvalidData, "Unsupported block mode"))
        };

        Ok(BlockHeader {
            size,
            grid_position,
            num_el: num_el as usize,
        })
    }
}

/// Reads blocks from rust readers.
pub trait DefaultBlockReader<T: ReflectedType, R: std::io::Read>: DefaultBlockHeaderReader<R> {
    fn read_block(
        mut buffer: R,
        data_attrs: &DatasetAttributes,
        grid_position: GridCoord,
    ) -> std::io::Result<VecDataBlock<T>>
            where VecDataBlock<T>: DataBlock<T> + ReadableDataBlock {

        if data_attrs.data_type != T::VARIANT {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Attempt to create data block for wrong type."))
        }
        let header = Self::read_block_header(&mut buffer, grid_position)?;

        let mut block = T::create_data_block(header);
        let mut decompressed = data_attrs.compression.decoder(buffer);
        block.read_data(&mut decompressed)?;

        Ok(block)
    }

    fn read_block_into<B: DataBlock<T> + ReinitDataBlock<T> + ReadableDataBlock>(
        mut buffer: R,
        data_attrs: &DatasetAttributes,
        grid_position: GridCoord,
        block: &mut B,
    ) -> std::io::Result<()> {

        if data_attrs.data_type != T::VARIANT {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Attempt to create data block for wrong type."))
        }
        let header = Self::read_block_header(&mut buffer, grid_position)?;

        block.reinitialize(header);
        let mut decompressed = data_attrs.compression.decoder(buffer);
        block.read_data(&mut decompressed)?;

        Ok(())
    }
}

/// Writes blocks to rust writers.
pub trait DefaultBlockWriter<T, W: std::io::Write, B: DataBlock<T> + WriteableDataBlock> {
    fn write_block(
        mut buffer: W,
        data_attrs: &DatasetAttributes,
        block: &B,
    ) -> std::io::Result<()> {
        let mode: u16 = if block.get_num_elements() == block.get_size().iter().product::<u32>()
            {0} else {1};
        buffer.write_u16::<BigEndian>(mode)?;
        buffer.write_u16::<BigEndian>(data_attrs.get_ndim() as u16)?;
        for i in block.get_size() {
            buffer.write_u32::<BigEndian>(*i)?;
        }

        if mode != 0 {
            buffer.write_u32::<BigEndian>(block.get_num_elements())?;
        }

        let mut compressor = data_attrs.compression.encoder(buffer);
        block.write_data(&mut compressor)?;

        Ok(())
    }
}

// TODO: needed because cannot invoke type parameterized static trait methods
// directly from trait name in Rust. Symptom of design problems with
// `DefaultBlockReader`, etc.
pub struct DefaultBlock;
impl<R: std::io::Read> DefaultBlockHeaderReader<R> for DefaultBlock {}
impl<T: ReflectedType, R: std::io::Read> DefaultBlockReader<T, R> for DefaultBlock {}
impl<T, W: std::io::Write, B: DataBlock<T> + WriteableDataBlock> DefaultBlockWriter<T, W, B> for DefaultBlock {}
