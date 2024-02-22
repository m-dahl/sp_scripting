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
    add_dump_function(&lua)?;
    add_sp_functions(&lua)?;

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

pub fn add_dump_function(lua: &Lua) -> Result<()> {
    // Helper function to debug print lua tables.
    lua.load(
        r#"
           function dump(t, indent, done)
               done = done or {}
               indent = indent or 0

               done[t] = true

               for key, value in pairs(t) do
                   spaces = string.rep(" ", indent)

                   if type(value) == "table" and not done[value] then
                       done[value] = true
                       print(spaces .. key .. ":")

                       dump(value, indent + 2, done)
                       done[value] = nil
                   else
                       print(spaces .. key .. " = " .. tostring(value))
                   end
               end
           end
       "#).exec()
}

pub fn add_sp_functions(lua: &Lua) -> Result<()> {
    // Helper function to debug print lua tables.
    lua.load(
        r#"
           sp = {}

           function sp.add_input(i)
             sp.inputs = sp.inputs or {}
             sp.inputs[i.lua_name] = i.redis_key
           end

           function sp.add_output(o)
             sp.outputs = sp.outputs or {}
             sp.outputs[o.lua_name] = { redis_key = o.redis_key, ttl = o.ttl or 10 }
           end

           function sp.tick()
              local fired, errors = sp.take_transitions()
              return #fired
           end

           function sp.take_transitions()
              local fired = {}
              local errors = {}

              -- run functions
              for _, f in pairs(sp.functions or {}) do
                 f()
              end

              -- run operations
              for name, o in pairs(sp.operations or {}) do
                 if o.state.state == "i" then
                    local status, result_or_error = pcall(o.start_guard, o.state)
                    if status and result_or_error then
                       o.state.state = "e"
                       o.start_action(o.state)
                       table.insert(fired, "start_" .. name)
                       print("Started " .. o.name)
                    elseif not status then
                       print("Operation error " .. o.name .. ": " .. result_or_error)
                       table.insert(errors, "start_" .. name .. ": " .. result_or_error)
                    end
                 elseif o.state.state == "e" then
                    local status, result_or_error = pcall(o.finish_guard, o.state)
                    if status and result_or_error then
                       o.state.state = "f"
                       o.finish_action(o.state)
                       table.insert(fired, "finish_" .. name)
                       print("Finished " .. o.name)
                    elseif not status then
                       print("Operation error " .. o.name .. ": " .. result_or_error)
                       table.insert(errors, "finish_" .. name .. ": " .. result_or_error)
                    end
                 elseif o.state.state == "f" then
                    local status, result_or_error = pcall(o.reset_guard, o.state)
                    if status and result_or_error then
                       o.state.state = "i"
                       o.reset_action(o.state)
                       table.insert(fired, "reset_" .. name)
                       print("Reset " .. o.name)
                    elseif not status then
                       table.insert(errors, "reset_" .. name .. ": " .. result_or_error)
                    end
                 end
              end

              return fired, errors
           end

           function sp.add_function(name, f)
              sp.functions = sp.functions or {}
              sp.functions[name] = f
           end

           function sp.add_operation(o)
              local o = o or {}
              o.state = { state = "i" }
              o.start_guard = o.start_guard or function (state) return false end
              o.start_action = o.start_action or function (state) end

              o.finish_guard = o.finish_guard or function (state) return true end
              o.finish_action = o.finish_action or function (state) end

              o.reset_guard = o.reset_guard or function (state) return false end
              o.reset_action = o.reset_action or function (state) end

              sp.operations = sp.operations or {}
              sp.operations[o.name] = o
           end

           function sp.remove_operation(name)
              if sp.operations and sp.operations[name] ~= nil then
                 sp.operations[name] = nil
              end
           end

       "#).exec()
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
