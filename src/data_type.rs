use serde::{
    Deserialize,
    Serialize,
};

use crate::BlockHeader;
use crate::VecDataBlock;


/// Data types representable in N5.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    UINT8,
    UINT16,
    UINT32,
    UINT64,
    INT8,
    INT16,
    INT32,
    INT64,
    FLOAT32,
    FLOAT64,
}

#[macro_export]
macro_rules! data_type_match {
    ($match_expr:ident, $ret:ty, $expr:block) => {
        {
            fn inner<RsType: $crate::ReflectedType>() -> $ret $expr;
            match $match_expr {
                $crate::DataType::UINT8 => inner::<u8>(),
                $crate::DataType::UINT16 => inner::<u16>(),
                $crate::DataType::UINT32 => inner::<u32>(),
                $crate::DataType::UINT64 => inner::<u64>(),
                $crate::DataType::INT8 => inner::<i8>(),
                $crate::DataType::INT16 => inner::<i16>(),
                $crate::DataType::INT32 => inner::<i32>(),
                $crate::DataType::INT64 => inner::<i64>(),
                $crate::DataType::FLOAT32 => inner::<f32>(),
                $crate::DataType::FLOAT64 => inner::<f64>(),
            }
        }
    };
}

impl DataType {
    /// Boilerplate method for reflection of primitive type sizes.
    pub fn size_of(self) -> usize {
        data_type_match!(self, usize, {
                std::mem::size_of::<RsType>()
            }
        )
    }
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub trait ReflectedType {
    const VARIANT: DataType;
}

/// Reflect rust types to type values.
pub trait TypeReflection<T> {
    fn get_type_variant() -> Self;
}

// TODO: replace this with a generic inherent function and instead check that
// dataset DataType is expected type (via `TypeReflection` trait).
pub trait DataBlockCreator<T: Clone> {
    fn create_data_block(
        &self,
        header: BlockHeader,
    ) -> Option<VecDataBlock<T>>;
}

macro_rules! data_type_block_creator {
    ($d_name:ident, $d_type:ty) => {
        impl TypeReflection<$d_type> for DataType {
            fn get_type_variant() -> DataType {
                DataType::$d_name
            }
        }

        impl ReflectedType for $d_type {
            const VARIANT: DataType = DataType::$d_name;
        }

        impl DataBlockCreator<$d_type> for DataType {
            fn create_data_block(
                &self,
                header: BlockHeader,
            ) -> Option<VecDataBlock<$d_type>> {
                match *self {
                    DataType::$d_name => Some(VecDataBlock::<$d_type>::new(
                        header.size,
                        header.grid_position,
                        vec![0. as $d_type; header.num_el],
                    )),
                    _ => None,
                }
            }
        }
    }
}

data_type_block_creator!(UINT8,  u8);
data_type_block_creator!(UINT16, u16);
data_type_block_creator!(UINT32, u32);
data_type_block_creator!(UINT64, u64);
data_type_block_creator!(INT8,  i8);
data_type_block_creator!(INT16, i16);
data_type_block_creator!(INT32, i32);
data_type_block_creator!(INT64, i64);
data_type_block_creator!(FLOAT32, f32);
data_type_block_creator!(FLOAT64, f64);
