use std::fs::File;
use std::io::{BufReader, Read};
use mlua::{chunk, Lua, Result, Value, Error, LuaSerdeExt, Function, Table};
use redis::Commands;

pub struct InputMapping {
    key: String,
    var: String,
}

pub struct OutputMapping {
    key: String,
    var: String,
    ttl: usize,
}

fn main() -> Result<()> {
    let lua = Lua::new();
    load_and_run(&lua, "src/utils.lua")?;
    load_and_run(&lua, "src/sp.lua")?;

    let r = redis::Client::open("redis://10.7.0.35/").unwrap();
    let mut rc = r.get_connection().unwrap();
    // Fetch the code and run it
    // let code: String = rc.get("sp/code").map_err(Error::external)?;
    // lua.load(&code).exec()?;
    load_and_run(&lua, "src/controller.lua")?;

    // Get input mappings
    let mut inputs = vec![];
    if let Ok(v) = lua.load("sp.inputs").eval::<Table>() {
        for pair in v.pairs::<String, String>() {
            let (key, value) = pair?;
            inputs.push(InputMapping {
                key: value,
                var: key,
            });
        }
    }

    // Get output mappings
    let mut outputs = vec![];
    if let Ok(v) = lua.load("sp.outputs").eval::<Table>() {
        for pair in v.pairs::<String, Table>() {
            let (key, value) = pair?;

            outputs.push(OutputMapping {
                key: value.get("redis_key")?,
                var: key,
                ttl: value.get("ttl")?,
            });
        }
    }

    loop {
        for i in &inputs {
            let r: std::result::Result<String, _> = rc.get(&i.key);
            if let Ok(g) = r {
                let value = serde_json::from_str::<serde_json::Value>(&g).map_err(Error::external)?;
                if let Some((parent, field)) = i.var.rsplit_once(".") {
                    let table = lua.load(parent).eval::<Table>()?;
                    table.set(field, lua.to_value(&value)?)?;
                } else {
                    // no split, assume global
                    lua.globals().set(i.var.clone(), lua.to_value(&value)?)?;
                }
            }
        }

        // Update with the current time.
        let now_millis = now().as_millis();
        lua.load(chunk! {
            sp.now = $now_millis
        }).exec()?;

        // Run tick function.
        let tick: Function = lua.load("sp.tick").eval()?;
        tick.call::<_, ()>(())?;

        for o in &outputs {
            if let Ok(v) = lua.load(&o.var).eval::<Value>() {
                let json = serde_json::to_string_pretty(&v).map_err(Error::external)?;
                let _r: std::result::Result<(), _> = rc.set_ex(&o.key, json, o.ttl);
            }
        }

        {
            // Save the entire sp state to redis.
            let jv = lua_table_to_json(lua.globals());
            let json = serde_json::to_string_pretty(&jv).map_err(Error::external)?;
            let _r: std::result::Result<(), _> = rc.set("sp/all", json);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
   }
}

pub fn load_and_run(lua: &Lua, filename: &str) -> Result<()> {
    let mut input: BufReader<File> = BufReader::new(File::open(filename)?);
    let mut s = String::new();
    input.read_to_string(&mut s)?;
    lua.load(s).exec()
}

pub fn lua_table_to_json<'lua>(t: Table<'lua>) -> serde_json::Value {
    fn is_serializable(v: &Value) -> bool {
        match v {
            Value::Nil => true,
            Value::Boolean(_) => true,
            Value::Integer(_) => true,
            Value::Number(_) => true,
            Value::String(_) => true,
            _ => false,
        }
    }

    fn inner<'lua>(table: Table<'lua>, seen: &mut Vec<(Table<'lua>, serde_json::Value)>) ->
        serde_json::Value {
        let mut map = serde_json::Map::new();
        for pair in table.clone().pairs::<String, Value>() {
            match pair {
                Ok((s, Value::Table(t))) if t != table => {
                    let tabval = if let Some(existing) = seen.iter().
                        find(|(tt, _)| *tt == t).map(|(_, existing)| existing) {
                            existing.clone()
                        } else {
                            let new = inner(t.clone(), seen);
                            seen.push((t, new.clone()));
                            new
                        };
                    map.insert(s, tabval);
                },
                Ok((s, t)) if is_serializable(&t) => {
                    let pod: serde_json::Value = serde_json::to_value(t).unwrap_or(serde_json::Value::Null);
                    map.insert(s, pod);
                },
                _ => {}
            }
        }
        return serde_json::Value::Object(map)
    }

    let mut seen = vec![];
    inner(t, &mut seen)
}


pub fn now() -> std::time::Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
}
