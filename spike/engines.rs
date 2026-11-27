use super::{fail, Result};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

pub struct Bridge {
    pub previous: Cell<i32>,
    pub pending: Cell<Option<i32>>,
    domains: Vec<String>,
    pub dispatched: Cell<u32>,
}
impl Bridge {
    pub fn new(domains: Vec<String>) -> Self {
        Self {
            previous: Cell::new(i32::MIN),
            pending: Cell::new(None),
            domains,
            dispatched: Cell::new(0),
        }
    }
    // 仅在权限检查之后进入确定性的内存传输端，测试机不会向公网发送请求。
    fn request(&self, value: &str) -> i32 {
        let Ok(url) = url::Url::parse(value) else {
            return -1;
        };
        if url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !self
                .domains
                .iter()
                .any(|d| Some(d.as_str()) == url.host_str())
        {
            return -1;
        }
        self.dispatched.set(self.dispatched.get() + 1);
        204
    }
}

pub enum Engine {
    #[cfg(feature = "sdk-js")]
    Js {
        ctx: rquickjs::Context,
        deadline: Rc<Cell<Instant>>,
    },
    #[cfg(feature = "sdk-wasm")]
    Wasm {
        store: wasmi::Store<(Rc<Bridge>, wasmi::StoreLimits)>,
        instance: wasmi::Instance,
    },
}
impl Engine {
    pub fn load(kind: &str, bytes: &[u8], bridge: Rc<Bridge>) -> Result<Self> {
        if bytes.len() > 256 * 1024 {
            return Err(fail("插件超过 256 KiB"));
        }
        match kind {
            #[cfg(feature = "sdk-js")]
            "js" => {
                use rquickjs::{Context, Function, Runtime};
                let rt = Runtime::new()?;
                rt.set_memory_limit(4 * 1024 * 1024);
                rt.set_max_stack_size(128 * 1024);
                let deadline = Rc::new(Cell::new(Instant::now() + Duration::from_millis(30)));
                let limit = deadline.clone();
                rt.set_interrupt_handler(Some(Box::new(move || Instant::now() >= limit.get())));
                let ctx = Context::full(&rt)?;
                ctx.with(|c| -> rquickjs::Result<()> {
                    let g = c.globals();
                    let b = bridge.clone();
                    g.set(
                        "lastDays",
                        Function::new(c.clone(), move || b.previous.get())?,
                    )?;
                    let b = bridge.clone();
                    g.set(
                        "saveDays",
                        Function::new(c.clone(), move |v: i32| b.pending.set(Some(v)))?,
                    )?;
                    g.set(
                        "request",
                        Function::new(c.clone(), move |url: String| bridge.request(&url))?,
                    )?;
                    c.eval::<(), _>(bytes.to_vec())
                })?;
                Ok(Self::Js { ctx, deadline })
            }
            #[cfg(feature = "sdk-wasm")]
            "wasm" => {
                use wasmi::{
                    Caller, Config, Engine, Extern, Linker, Module, Store, StoreLimits,
                    StoreLimitsBuilder,
                };
                type Data = (Rc<Bridge>, StoreLimits);
                let mut cfg = Config::default();
                cfg.consume_fuel(true);
                let engine = Engine::new(&cfg);
                let module = Module::new(&engine, bytes)?;
                let limits = StoreLimitsBuilder::new()
                    .memory_size(4 * 1024 * 1024)
                    .memories(1)
                    .tables(1)
                    .table_elements(1024)
                    .instances(1)
                    .trap_on_grow_failure(true)
                    .build();
                let mut store = Store::new(&engine, (bridge, limits));
                store.limiter(|data| &mut data.1);
                store.set_fuel(100_000)?;
                let mut linker = Linker::<Data>::new(&engine);
                linker.func_wrap("sdk", "last_days", |c: Caller<Data>| {
                    c.data().0.previous.get()
                })?;
                linker.func_wrap("sdk", "save_days", |c: Caller<Data>, v: i32| {
                    c.data().0.pending.set(Some(v))
                })?;
                linker.func_wrap("sdk", "request", |c: Caller<Data>, ptr: u32, len: u32| {
                    if len > 2048 {
                        return -1;
                    }
                    let Some(Extern::Memory(mem)) = c.get_export("memory") else {
                        return -1;
                    };
                    let mut buf = [0u8; 2048];
                    let out = &mut buf[..len as usize];
                    if mem.read(&c, ptr as usize, out).is_err() {
                        return -1;
                    }
                    std::str::from_utf8(out).map_or(-1, |url| c.data().0.request(url))
                })?;
                let instance = linker.instantiate(&mut store, &module)?.start(&mut store)?;
                Ok(Self::Wasm { store, instance })
            }
            _ => Err(fail("构建没有启用所选运行环境")),
        }
    }
    pub fn call(&mut self, target: i32, today: i32, fault: Option<i32>) -> Result<i32> {
        match self {
            #[cfg(feature = "sdk-js")]
            Self::Js { ctx, deadline } => {
                deadline.set(Instant::now() + Duration::from_millis(30));
                Ok(ctx.with(|c| -> rquickjs::Result<i32> {
                    if let Some(n) = fault {
                        c.globals()
                            .get::<_, rquickjs::Function>("fault")?
                            .call((n,))
                    } else {
                        c.globals()
                            .get::<_, rquickjs::Function>("refresh")?
                            .call((target, today))
                    }
                })?)
            }
            #[cfg(feature = "sdk-wasm")]
            Self::Wasm { store, instance } => {
                store.set_fuel(100_000)?;
                if let Some(n) = fault {
                    Ok(instance
                        .get_typed_func::<i32, i32>(&*store, "fault")?
                        .call(&mut *store, n)?)
                } else {
                    Ok(instance
                        .get_typed_func::<(i32, i32), i32>(&*store, "refresh")?
                        .call(&mut *store, (target, today))?)
                }
            }
        }
    }
}
