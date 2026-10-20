//! Synchronous Python bindings. Query execution releases the GIL; each session
//! serializes its own operations while independent sessions share the database.
use graphfusion::{
    arrow::ipc::writer::StreamWriter, Database, OpenOptions, QueryLimits, Session, StatementOutput,
    Value,
};
use pyo3::{
    create_exception,
    exceptions::{PyException, PyRuntimeError, PyTypeError},
    prelude::*,
    types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple},
};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

create_exception!(_native, GraphFusionError, PyException);

fn engine_error(error: impl std::fmt::Display) -> PyErr {
    GraphFusionError::new_err(error.to_string())
}

#[pyclass(name = "Database", module = "graphfusion._native")]
struct PyDatabase {
    inner: Option<Database>,
}

#[pymethods]
impl PyDatabase {
    #[new]
    #[pyo3(signature = (path=None, *, create=false))]
    fn new(py: Python<'_>, path: Option<PathBuf>, create: bool) -> PyResult<Self> {
        let db = py
            .detach(move || match path {
                Some(path) => Database::open(
                    path,
                    OpenOptions {
                        create_if_missing: create,
                    },
                ),
                None => Ok(Database::new()),
            })
            .map_err(engine_error)?;
        Ok(Self { inner: Some(db) })
    }

    fn session(&self) -> PyResult<PySession> {
        let db = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("database is closed"))?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(engine_error)?;
        Ok(PySession {
            inner: Mutex::new(Some(db.session())),
            runtime,
        })
    }

    fn checkpoint(&self, py: Python<'_>) -> PyResult<()> {
        let db = self
            .inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("database is closed"))?;
        py.detach(|| db.checkpoint()).map_err(engine_error)
    }

    fn close(&mut self) {
        self.inner = None;
    }
}

#[pyclass(name = "Session", module = "graphfusion._native")]
struct PySession {
    inner: Mutex<Option<Session>>,
    runtime: tokio::runtime::Runtime,
}

impl PySession {
    fn with_session<T>(&self, action: impl FnOnce(&mut Session) -> PyResult<T>) -> PyResult<T> {
        let mut guard = self.inner.lock().map_err(engine_error)?;
        action(
            guard
                .as_mut()
                .ok_or_else(|| PyRuntimeError::new_err("session is closed"))?,
        )
    }
}

#[pymethods]
impl PySession {
    fn run(&self, py: Python<'_>, gql: String) -> PyResult<Vec<PyResultSet>> {
        py.detach(|| {
            self.with_session(|session| {
                self.runtime
                    .block_on(session.run(&gql))
                    .map_err(engine_error)?
                    .into_iter()
                    .map(convert_output)
                    .collect()
            })
        })
    }

    fn query(&self, py: Python<'_>, gql: String) -> PyResult<PyResultSet> {
        py.detach(|| {
            self.with_session(|session| {
                let result = self
                    .runtime
                    .block_on(session.query(&gql))
                    .map_err(engine_error)?;
                convert_output(StatementOutput::Query(result))
            })
        })
    }

    fn set_parameter(
        &self,
        py: Python<'_>,
        name: String,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let value = parameter(value, 0)?;
        py.detach(|| self.with_session(|s| s.set_parameter(name, value).map_err(engine_error)))
    }

    fn set_query_limits(
        &self,
        py: Python<'_>,
        max_path_hops: u64,
        memory_limit_bytes: usize,
    ) -> PyResult<()> {
        py.detach(|| {
            self.with_session(|s| {
                s.set_query_limits(QueryLimits {
                    max_path_hops,
                    memory_limit_bytes,
                })
                .map_err(engine_error)
            })
        })
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| {
            self.inner.lock().map_err(engine_error)?.take();
            Ok(())
        })
    }
}

#[pyclass(module = "graphfusion._native")]
struct PyResultSet {
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    commit_seq: u64,
    #[pyo3(get)]
    affected: usize,
    #[pyo3(get)]
    transaction_pending: bool,
    ipc: Option<Vec<u8>>,
}

#[pymethods]
impl PyResultSet {
    fn arrow_ipc<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.ipc.as_ref().map(|data| PyBytes::new(py, data))
    }
}

fn convert_output(output: StatementOutput) -> PyResult<PyResultSet> {
    match output {
        StatementOutput::Command(r) => Ok(PyResultSet {
            kind: "command",
            commit_seq: r.commit_seq,
            affected: r.affected_objects,
            transaction_pending: r.transaction_pending,
            ipc: None,
        }),
        StatementOutput::Query(r) => {
            let mut data = Vec::new();
            {
                let mut writer =
                    StreamWriter::try_new(&mut data, &r.schema).map_err(engine_error)?;
                for batch in &r.batches {
                    writer.write(batch).map_err(engine_error)?;
                }
                writer.finish().map_err(engine_error)?;
            }
            Ok(PyResultSet {
                kind: "query",
                commit_seq: r.commit_seq,
                affected: r.affected_elements,
                transaction_pending: r.transaction_pending,
                ipc: Some(data),
            })
        }
    }
}

fn parameter(value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
    if depth > 64 {
        return Err(PyTypeError::new_err("parameter nesting exceeds 64 levels"));
    }
    Ok(if value.is_none() {
        Value::Null
    } else if value.is_instance_of::<PyBool>() {
        Value::Boolean(value.extract()?)
    } else if value.is_instance_of::<PyInt>() {
        Value::Integer(value.extract()?)
    } else if value.is_instance_of::<PyFloat>() {
        Value::Float(value.extract()?)
    } else if value.is_instance_of::<PyString>() {
        Value::String(value.extract()?)
    } else if value.is_instance_of::<PyBytes>() {
        Value::Bytes(value.extract()?)
    } else if let Ok(dict) = value.cast::<PyDict>() {
        let mut fields = BTreeMap::new();
        for (key, value) in dict {
            fields.insert(key.extract::<String>()?, parameter(&value, depth + 1)?);
        }
        Value::Record(fields)
    } else if let Ok(list) = value.cast::<PyList>() {
        Value::List(
            list.iter()
                .map(|v| parameter(&v, depth + 1))
                .collect::<PyResult<_>>()?,
        )
    } else if let Ok(tuple) = value.cast::<PyTuple>() {
        Value::List(
            tuple
                .iter()
                .map(|v| parameter(&v, depth + 1))
                .collect::<PyResult<_>>()?,
        )
    } else {
        return Err(PyTypeError::new_err("parameters support None, bool, int64, float, str, bytes, list, tuple and dict[str, value]"));
    })
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDatabase>()?;
    m.add_class::<PySession>()?;
    m.add_class::<PyResultSet>()?;
    m.add("GraphFusionError", m.py().get_type::<GraphFusionError>())?;
    Ok(())
}
