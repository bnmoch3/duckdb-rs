use crate::{error::Error, inner_connection::InnerConnection, Connection, Result};

use super::{ffi, ffi::duckdb_free};
use std::ffi::c_void;

mod function;
mod value;

/// The duckdb Arrow table function interface
#[cfg(feature = "vtab-arrow")]
pub mod arrow;
#[cfg(feature = "vtab-arrow")]
pub use self::arrow::{
    arrow_arraydata_to_query_params, arrow_ffi_to_query_params, arrow_recordbatch_to_query_params,
    record_batch_to_duckdb_data_chunk, to_duckdb_logical_type, to_duckdb_type_id,
};
#[cfg(feature = "vtab-excel")]
mod excel;

pub use function::{BindInfo, FunctionInfo, InitInfo, TableFunction};
pub use value::Value;

use crate::core::{DataChunkHandle, LogicalTypeHandle};
use ffi::{duckdb_bind_info, duckdb_data_chunk, duckdb_function_info, duckdb_init_info};

use ffi::duckdb_malloc;
use std::mem::size_of;

/// duckdb_malloc a struct of type T
/// used for the bind_info and init_info
/// # Safety
/// This function is obviously unsafe
unsafe fn malloc_data_c<T>() -> *mut T {
    duckdb_malloc(size_of::<T>()).cast()
}

/// free bind or info data
///
/// # Safety
///   This function is obviously unsafe
/// TODO: maybe we should use a Free trait here
unsafe extern "C" fn drop_data_c<T: Free>(v: *mut c_void) {
    let actual = v.cast::<T>();
    (*actual).free();
    duckdb_free(v);
}

/// Free trait for the bind and init data
pub trait Free {
    /// Free the data
    fn free(&mut self) {}
}

/// Duckdb table function trait
///
/// See to the HelloVTab example for more details
/// <https://duckdb.org/docs/api/c/table_functions>
pub trait VTab: Sized {
    /// The data type of the bind data
    type BindData: Sized + Free;
    /// The data type of the global state data
    type GlobalData: Sized + Free;

    /// Bind data to the table function
    ///
    /// # Safety
    ///
    /// This function is unsafe because it dereferences raw pointers (`data`) and manipulates the memory directly.
    /// The caller must ensure that:
    ///
    /// - The `data` pointer is valid and points to a properly initialized `BindData` instance.
    /// - The lifetime of `data` must outlive the execution of `bind` to avoid dangling pointers, especially since
    ///   `bind` does not take ownership of `data`.
    /// - Concurrent access to `data` (if applicable) must be properly synchronized.
    /// - The `bind` object must be valid and correctly initialized.
    unsafe fn bind(bind: &BindInfo, data: *mut Self::BindData) -> Result<(), Box<dyn std::error::Error>>;
    /// Initialize the table function's global data
    ///
    /// # Safety
    ///
    /// This function is unsafe because it performs raw pointer dereferencing on the `data` argument.
    /// The caller is responsible for ensuring that:
    ///
    /// - The `data` pointer is non-null and points to a valid `GlobalData` instance.
    /// - `data` can be accessed from multiple threads which means proper synchronization is required.
    /// - The lifetime of `data` extends beyond the scope of this call to avoid use-after-free errors.
    unsafe fn init(init: &InitInfo, data: *mut Self::GlobalData) -> Result<(), Box<dyn std::error::Error>>;
    /// The actual function
    ///
    /// # Safety
    ///
    /// This function is unsafe because it:
    ///
    /// - Dereferences multiple raw pointers (`func` to access `init_info` and `bind_info`).
    ///
    /// The caller must ensure that:
    ///
    /// - All pointers (`func`, `output`, internal `init_info`, and `bind_info`) are valid and point to the expected types of data structures.
    /// - The `init_info` and `bind_info` data pointed to remains valid and is not freed until after this function completes.
    /// - No other threads are concurrently mutating the data pointed to by `init_info` and `bind_info` without proper synchronization.
    /// - The `output` parameter is correctly initialized and can safely be written to.
    unsafe fn func(func: &FunctionInfo, output: &mut DataChunkHandle) -> Result<(), Box<dyn std::error::Error>>;
    /// Does the table function support pushdown
    /// default is false
    fn supports_pushdown() -> bool {
        false
    }
    /// The parameters of the table function
    /// default is None
    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        None
    }
    /// The named parameters of the table function
    /// default is None
    fn named_parameters() -> Option<Vec<(String, LogicalTypeHandle)>> {
        None
    }

    /// The name of the table function. E.g if name returns `hello` then in
    /// DuckDB once the table function is loaded it can be invoked as:
    /// `select * from hello(...)`
    fn name() -> &'static str;
}

/// DuckDB table function trait for  table functions that use local state per
/// worker thread
pub trait VTabWithLocalData: VTab {
    /// The data type for local state data
    type LocalData: Sized + Free;

    /// Initialize the table function's local data
    ///
    /// # Safety
    ///
    /// This function is unsafe because it performs raw pointer dereferencing on the `data` argument.
    /// The caller is responsible for ensuring that:
    ///
    /// - The `data` pointer is non-null and points to a valid `LocalData` instance.
    /// - The lifetime of `data` extends beyond the scope of this call to avoid use-after-free errors.
    unsafe fn init_local(_init: &InitInfo, _data: *mut Self::LocalData) -> Result<(), Box<dyn std::error::Error>>;
}

unsafe extern "C" fn func<T>(info: duckdb_function_info, output: duckdb_data_chunk)
where
    T: VTab,
{
    let info = FunctionInfo::from(info);
    let mut data_chunk_handle = DataChunkHandle::new_unowned(output);
    let result = T::func(&info, &mut data_chunk_handle);
    if result.is_err() {
        info.set_error(&result.err().unwrap().to_string());
    }
}

unsafe extern "C" fn init<T>(info: duckdb_init_info)
where
    T: VTab,
{
    let info = InitInfo::from(info);
    let data = malloc_data_c::<T::GlobalData>();
    let result = T::init(&info, data);
    info.set_init_data(data.cast(), Some(drop_data_c::<T::GlobalData>));
    if result.is_err() {
        info.set_error(&result.err().unwrap().to_string());
    }
}

unsafe extern "C" fn local_init<T>(info: duckdb_init_info)
where
    T: VTabWithLocalData,
{
    let info = InitInfo::from(info);
    let data = malloc_data_c::<T::LocalData>();
    let result = T::init_local(&info, data);
    info.set_init_data(data.cast(), Some(drop_data_c::<T::LocalData>));
    if result.is_err() {
        info.set_error(&result.err().unwrap().to_string());
    }
}

unsafe extern "C" fn bind<T>(info: duckdb_bind_info)
where
    T: VTab,
{
    let info = BindInfo::from(info);
    let data = malloc_data_c::<T::BindData>();
    let result = T::bind(&info, data);
    info.set_bind_data(data.cast(), Some(drop_data_c::<T::BindData>));
    if result.is_err() {
        info.set_error(&result.err().unwrap().to_string());
    }
}

impl Connection {
    /// Register the given TableFunction with the current db iff it does not
    /// need to use local state and only accesses global state i.e. it uses max
    /// 1 thread (single-threaded)
    #[inline]
    pub fn register_table_function<T: VTab>(&self) -> Result<()> {
        let table_function = into_table_function::<T>();
        self.db.borrow_mut().register_table_function(table_function)
    }

    #[inline]
    /// Register the given TableFunction with the current db iff it needs to
    /// init local state per worker thread (max threads set to > 1)
    pub fn register_table_function_with_local_init<T: VTabWithLocalData>(&self) -> Result<()> {
        let table_function = into_table_function::<T>();
        table_function.set_local_init(Some(local_init::<T>));
        self.db.borrow_mut().register_table_function(table_function)
    }
}

impl InnerConnection {
    /// Register the given TableFunction with the current db
    pub fn register_table_function(&mut self, table_function: TableFunction) -> Result<()> {
        unsafe {
            let rc = ffi::duckdb_register_table_function(self.con, table_function.ptr);
            if rc != ffi::DuckDBSuccess {
                return Err(Error::DuckDBFailure(ffi::Error::new(rc), None));
            }
        }
        Ok(())
    }
}
fn into_table_function<T: VTab>() -> TableFunction {
    let table_function = TableFunction::default();
    table_function
        .set_name(T::name())
        .supports_pushdown(T::supports_pushdown())
        .set_bind(Some(bind::<T>))
        .set_init(Some(init::<T>))
        .set_function(Some(func::<T>));
    for ty in T::parameters().unwrap_or_default() {
        table_function.add_parameter(&ty);
    }
    for (name, ty) in T::named_parameters().unwrap_or_default() {
        table_function.add_named_parameter(&name, &ty);
    }

    table_function
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        core::{Inserter, LogicalTypeId},
        params,
    };
    use std::{
        error::Error,
        ffi::{c_char, CString},
    };

    #[derive(Debug)]
    #[repr(C)]
    struct HelloBindData {
        name: *mut c_char,
        count: u32,
    }

    impl Free for HelloBindData {
        fn free(&mut self) {
            unsafe {
                if self.name.is_null() {
                    return;
                }
                drop(CString::from_raw(self.name));
            }
        }
    }

    #[repr(C)]
    struct HelloGlobalData {
        done: bool,
    }

    impl Free for HelloGlobalData {}

    #[repr(C)]
    #[derive(Debug)]
    struct HelloLocalData {
        remaining: u32,
    }

    impl Free for HelloLocalData {}

    struct HelloVTab;

    impl VTab for HelloVTab {
        type BindData = HelloBindData;
        type GlobalData = HelloGlobalData;

        unsafe fn bind(info: &BindInfo, bind_data: *mut HelloBindData) -> Result<(), Box<dyn std::error::Error>> {
            // params
            let bind_data = unsafe { &mut *bind_data };
            let name_param = info.get_parameter(0).to_string();
            let count_param = info.get_named_parameter("count").unwrap().to_int64();
            bind_data.name = CString::new(name_param).unwrap().into_raw();
            bind_data.count = count_param.try_into()?;

            // schema
            info.add_result_column("column0", LogicalTypeHandle::from(LogicalTypeId::Varchar));
            Ok(())
        }

        unsafe fn init(_: &InitInfo, data: *mut HelloGlobalData) -> Result<(), Box<dyn std::error::Error>> {
            let data = unsafe { &mut *data };
            data.done = false;
            Ok(())
        }

        unsafe fn func(func: &FunctionInfo, output: &mut DataChunkHandle) -> Result<(), Box<dyn std::error::Error>> {
            let bind_info = &mut *func.get_bind_data::<HelloBindData>();
            let global_state = &mut *func.get_init_data::<HelloGlobalData>();
            let local_state = &mut *func.get_local_init_data::<HelloLocalData>();

            if global_state.done {
                output.set_len(0);
            } else {
                let names_vec = output.flat_vector(0);
                let name = CString::from_raw(bind_info.name);
                let result = CString::new(format!("Hello {} {}", name.to_str()?, local_state.remaining))?;
                bind_info.name = CString::into_raw(name);
                names_vec.insert(0, result);
                output.set_len(1);
                local_state.remaining -= 1;
                if local_state.remaining == 0 {
                    global_state.done = true;
                }
            }
            Ok(())
        }

        fn parameters() -> Option<Vec<LogicalTypeHandle>> {
            Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
        }

        fn named_parameters() -> Option<Vec<(String, LogicalTypeHandle)>> {
            Some(vec![(
                "count".to_string(),
                LogicalTypeHandle::from(LogicalTypeId::Bigint),
            )])
        }

        fn name() -> &'static str {
            "hello"
        }
    }

    impl VTabWithLocalData for HelloVTab {
        type LocalData = HelloLocalData;
        unsafe fn init_local(
            init_info: &InitInfo,
            local_data: *mut HelloLocalData,
        ) -> Result<(), Box<dyn std::error::Error>> {
            let bind_data: &HelloBindData = unsafe { &*init_info.get_bind_data() };
            let local_data = unsafe { &mut *local_data };
            local_data.remaining = bind_data.count;
            Ok(())
        }
    }

    #[test]
    fn test_table_function() -> Result<(), Box<dyn Error>> {
        let conn = Connection::open_in_memory()?;
        conn.register_table_function_with_local_init::<HelloVTab>()?;
        let table_function_name = HelloVTab::name();

        let person = "Alice";
        let count: i64 = 10;
        let got = conn.query_row(
            &format!("select count(*) from {table_function_name}(?, count=?)"),
            params![person, count],
            |row| <(i64,)>::try_from(row),
        )?;
        assert_eq!(count, got.0);
        Ok(())
    }

    #[cfg(feature = "vtab-loadable")]
    use duckdb_loadable_macros::duckdb_entrypoint;

    // this function is never called, but is still type checked
    // Exposes a extern C function named "libhello_ext_init" in the compiled dynamic library,
    // the "entrypoint" that duckdb will use to load the extension.
    #[cfg(feature = "vtab-loadable")]
    #[duckdb_entrypoint]
    fn libhello_ext_init(conn: Connection) -> Result<(), Box<dyn Error>> {
        conn.register_table_function::<HelloVTab>()?;
        Ok(())
    }
}
