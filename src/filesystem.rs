use std::fs::{
    self,
    File,
};
use std::io::{
    Error,
    ErrorKind,
    BufReader,
    BufWriter,
    Read,
    Result,
    Seek,
    SeekFrom,
};
use std::path::{
    PathBuf,
};
use std::str::FromStr;

use fs2::FileExt;
use serde_json::{
    self,
    Value,
};

use ::{
    DataBlock,
    DataBlockCreator,
    DataType,
    DatasetAttributes,
    DefaultBlockReader,
    DefaultBlockWriter,
    N5Reader,
    N5Writer,
    Version,
};


const ATTRIBUTES_FILE: &str = "attributes.json";


pub struct N5Filesystem {
    base_path: PathBuf,
}

impl N5Filesystem {
    pub fn open(base_path: &str) -> Result<N5Filesystem> {
        let reader = N5Filesystem {
            base_path: PathBuf::from(base_path),
        };

        if reader.exists("") {
            let version = reader.get_version()?;

            if !::VERSION.is_compatible(&version) {
                return Err(Error::new(ErrorKind::Other, "TODO: Incompatible version"))
            }
        }

        Ok(reader)
    }

    pub fn open_or_create(base_path: &str) -> Result<N5Filesystem> {
        let reader = N5Filesystem {
            base_path: PathBuf::from(base_path),
        };

        fs::create_dir_all(base_path)?;

        if reader.get_version().map(|v| !v.is_compatible(&::VERSION)).unwrap_or(false) {
            return Err(Error::new(ErrorKind::Other, "TODO: Incompatible version"))
        } else {
            reader.set_attribute("", ::VERSION_ATTRIBUTE_KEY.to_owned(), ::VERSION.to_string())?;
        }

        Ok(reader)
    }

    pub fn get_attributes(&self, path_name: &str) -> Result<Value> {
        if self.exists(path_name) {
            let attr_path = self.base_path.join(path_name).join(ATTRIBUTES_FILE);

            if attr_path.exists() && attr_path.is_file() {
                let file = File::open(attr_path)?;
                file.lock_shared()?;
                let reader = BufReader::new(file);
                Ok(serde_json::from_reader(reader)?)
            } else {
                Ok(json!({}))
            }
        } else {
            Err(Error::new(ErrorKind::NotFound, "Path does not exist"))
        }
    }

    fn get_path(&self, path_name: &str) -> Result<PathBuf> {
        // Note: cannot use `canonicalize` on both the constructed dataset path
        // and `base_path` and check `starts_with`, because `canonicalize` also
        // requires the path exist.
        use std::path::Component;

        // TODO: cleanup?
        let data_path = PathBuf::from(path_name);
        if data_path.is_relative() {
            let mut nest: i32 = 0;
            let mut interior = true;
            for component in data_path.components() {
                match component {
                    Component::Prefix(_) => unreachable!(), // Not an absolute path.
                    Component::RootDir => unreachable!(), // Not an absolute path.
                    Component::CurDir => continue,
                    Component::ParentDir => nest -= 1,
                    Component::Normal(_) => nest += 1,
                };

                if nest < 0 {
                    interior = false
                }
            }

            if interior {
                return Ok(self.base_path.join(path_name))
            }
        }

        Err(Error::new(ErrorKind::NotFound, "Path name is outside this N5 filesystem"))
    }

    fn get_data_block_path(&self, path_name: &str, grid_position: &[i64]) -> Result<PathBuf> {
        let mut path = self.get_path(path_name)?;
        for coord in grid_position {
            path.push(coord.to_string());
        }
        Ok(path)
    }

    fn get_attributes_path(&self, path_name: &str) -> Result<PathBuf> {
        let mut path = self.get_path(path_name)?;
        path.push(ATTRIBUTES_FILE);
        Ok(path)
    }
}

impl N5Reader for N5Filesystem {
    fn get_version(&self) -> Result<Version> {
        // TODO: dedicated error type should clean this up.
        Ok(Version::from_str(self
                .get_attributes("")?
                .get(::VERSION_ATTRIBUTE_KEY)
                    .ok_or_else(|| Error::new(ErrorKind::NotFound, "Version attribute not present"))?
                .as_str().unwrap_or("")
            ).unwrap())
    }

    fn get_dataset_attributes(&self, path_name: &str) -> Result<DatasetAttributes> {
        let attr_path = self.get_attributes_path(path_name)?;
        let reader = BufReader::new(File::open(attr_path)?);
        Ok(serde_json::from_reader(reader)?)
    }

    fn exists(&self, path_name: &str) -> bool {
        let target = self.base_path.join(path_name);
        target.is_dir()
    }

    fn read_block<T>(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
        grid_position: Vec<i64>
    ) -> Result<Option<Box<DataBlock<Vec<T>>>>>
            where DataType: DataBlockCreator<Vec<T>> {
        let block_file = self.get_data_block_path(path_name, &grid_position)?;
        if block_file.is_file() {
            let file = File::open(block_file)?;
            file.lock_shared()?;
            let reader = BufReader::new(file);
            Ok(Some(<::Foo as DefaultBlockReader<T, _>>::read_block(
                reader,
                data_attrs,
                grid_position)?))
        } else {
            Ok(None)
        }
    }

    fn list(&self, path_name: &str) -> Result<Vec<String>> {
        // TODO: shouldn't do this in a closure to not equivocate errors with Nones.
        Ok(fs::read_dir(self.get_path(path_name)?)?
            .filter_map(|e| {
                if let Ok(file) = e {
                    if file.file_type().map(|f| f.is_dir()).ok() == Some(true) {
                        file.file_name().into_string().ok()
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect())
    }

    // TODO: dupe with get_attributes w/ different empty behaviors
    fn list_attributes(&self, path_name: &str) -> Result<Value> {
        let attr_path = self.get_attributes_path(path_name)?;
        let file = File::open(attr_path)?;
        file.lock_shared()?;
        let reader = BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    }
}

// From: https://github.com/serde-rs/json/issues/377
// TODO: Could be much better.
fn merge(a: &mut Value, b: &Value) {
    match (a, b) {
        (&mut Value::Object(ref mut a), &Value::Object(ref b)) => {
            for (k, v) in b {
                merge(a.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (a, b) => {
            *a = b.clone();
        }
    }
}

impl N5Writer for N5Filesystem {
    fn set_attributes(
        &self,
        path_name: &str,
        attributes: serde_json::Map<String, Value>,
    ) -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(self.get_attributes_path(path_name)?)?;
        file.lock_exclusive()?;

        let mut existing_buf = String::new();
        file.read_to_string(&mut existing_buf)?;
        file.seek(SeekFrom::Start(0))?;
        let existing = serde_json::from_str(&existing_buf).unwrap_or_else(|_| json!({}));
        let mut merged = existing.clone();

        let new: Value = attributes.into();

        merge(&mut merged, &new);

        if new != existing {
            let writer = BufWriter::new(file);
            serde_json::to_writer(writer, &merged)?;
        }

        Ok(())
    }

    fn create_group(&self, path_name: &str) -> Result<()> {
        let path = self.get_path(path_name)?;
        fs::create_dir_all(path)
    }

    fn write_block<T>(
        &self,
        path_name: &str,
        data_attrs: &DatasetAttributes,
        block: Box<DataBlock<T>>,
    ) -> Result<()> {
        let path = self.get_data_block_path(path_name, block.get_grid_position())?;
        fs::create_dir_all(path.parent().expect("TODO: root block path?"))?;

        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;
        file.lock_exclusive()?;

        let buffer = BufWriter::new(file);
        <::Foo as DefaultBlockWriter<T, _>>::write_block(
                buffer,
                data_attrs,
                block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    #[test]
    fn create_filesystem() {
        let dir = TempDir::new("rust_n5_tests").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let create = N5Filesystem::open_or_create(path_str)
            .expect("Failed to create N5 filesystem");
        create.set_attribute("", "foo".to_owned(), "bar")
            .expect("Failed to set attribute");

        let read = N5Filesystem::open(path_str)
            .expect("Failed to open N5 filesystem");

        assert_eq!(read.get_version().expect("Cannot read version"), *::VERSION);
        assert_eq!(read.list_attributes("").unwrap()["foo"], "bar");
    }

    #[test]
    fn create_dataset() {
        let dir = TempDir::new("rust_n5_tests").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let create = N5Filesystem::open_or_create(path_str)
            .expect("Failed to create N5 filesystem");
        let data_attrs = DatasetAttributes::new(
            vec![10, 10, 10],
            vec![5, 5, 5],
            DataType::INT32,
            ::compression::CompressionType::Raw(::compression::raw::RawCompression::default()),
        );
        create.create_dataset("foo/bar", &data_attrs)
            .expect("Failed to create dataset");

        let read = N5Filesystem::open(path_str)
            .expect("Failed to open N5 filesystem");

        assert_eq!(read.get_dataset_attributes("foo/bar").unwrap(), data_attrs);
    }

    #[test]
    fn reject_exterior_paths() {
        let dir = TempDir::new("rust_n5_tests").unwrap();
        let path_str = dir.path().to_str().unwrap();

        let create = N5Filesystem::open_or_create(path_str)
            .expect("Failed to create N5 filesystem");

        assert!(create.get_path("/").is_err());
        assert!(create.get_path("..").is_err());
        assert!(create.get_path("foo/bar/baz/../../..").is_ok());
        assert!(create.get_path("foo/bar/baz/../../../..").is_err());
    }

    #[test]
    fn create_block_rw() {
        let dir = TempDir::new("rust_n5_tests").unwrap();
        let path_str = dir.path().to_str().unwrap();
        // let path_str = "tmp";

        let create = N5Filesystem::open_or_create(path_str)
            .expect("Failed to create N5 filesystem");
        let data_attrs = DatasetAttributes::new(
            vec![10, 10, 10],
            vec![5, 5, 5],
            DataType::INT32,
            ::compression::CompressionType::Raw(::compression::raw::RawCompression::default()),
        );
        let block_data: Vec<i32> = (0..125_i32).collect();
        let block_in = Box::new(::VecDataBlock::new(
            data_attrs.block_size.clone(),
            vec![0, 0, 0],
            block_data.clone()));

        create.create_dataset("foo/bar", &data_attrs)
            .expect("Failed to create dataset");
        create.write_block("foo/bar", &data_attrs, block_in)
            .expect("Failed to write block");

        let read = N5Filesystem::open(path_str)
            .expect("Failed to open N5 filesystem");
        let block_out = read.read_block::<i32>("foo/bar", &data_attrs, vec![0, 0, 0])
            .expect("Failed to read block")
            .expect("Block is empty");
        let missing_block_out = read.read_block::<i32>("foo/bar", &data_attrs, vec![0, 0, 1])
            .expect("Failed to read block");

        assert_eq!(block_out.get_data(), &block_data);
        assert!(missing_block_out.is_none());
    }
}