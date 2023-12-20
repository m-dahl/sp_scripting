use mlua::{chunk, Lua, Result, Value, Error, LuaSerdeExt, Function, Table};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    parameter1: String,
    parameter2: f64,
}

#[derive(Serialize, Deserialize, Debug)]
struct Robot {
    active: bool,
    state: i32,
    config: Config,
}

fn main() -> Result<()> {
    let lua = Lua::new();
    add_dump_function(&lua)?;
    let globals = lua.globals();

    // Write robot1 to lua land
    let robot1 = Robot {
        active: true,
        state: 1,
        config: Config {
            parameter1: "something".into(),
            parameter2: 5.0,
        }
    };

    globals.set("robot1", lua.to_value(&robot1)?)?;

    // robot2 is created in lua land
    lua.load(r#"robot2 = {active = true, state = 1, config = { parameter1 = "other", parameter2 = 10.0 }}"#)
        .eval()?;

    let robot2: Robot = lua.from_value(globals.get("robot2")?).map_err(Error::external)?;
    println!("robot2: {:?}", robot2);

    // Run some transitions.

    // eval a guard
    let guard: bool = lua.load("robot1.active or robot2.active").eval()?;
    println!("result: {guard}");

    // eval an action
    lua.load("robot1.active = not robot1.active").eval()?;

    // lua transition helpers
    lua.load(chunk! {
        function add_transition(t)
            table.insert(transitions, t)
        end
        function run_transitions()
            for i, t in ipairs(transitions) do
              if t.guard() then
//                  print("taking " .. t.name)
                  t.action()
              end
            end
        end
        transitions = {}
        scratch = {counter = 0}
    }).exec()?;

    // Make a sp model with two transitions
    lua.load(chunk! {
        add_transition{name = "t1",
                       guard = function () return not robot1.active end,
                       action = function () robot1.active = true; scratch.counter += 1 return end
        }
        add_transition{name = "t2",
                       guard = function () return robot1.active end,
                       action = function () robot1.active = false; scratch.counter += 1 return end
        }
    }).exec()?;

    // Run a million cycles in lua land
    let t = now();
    lua.load(chunk! {
        for i=1,1000000 do
            run_transitions()
        end
    }).exec()?;
    let passed = now() - t;
    println!("Lua evaluation took: {}ms", passed.as_millis());

    // Or we can run the transitions from rust land instead.
    // (but predicates and actions are still evaluated in lua land)
    // To see how much overhead we have.
    pub struct Transition<'a> {
        pub name: String,
        pub guard: Function<'a>,
        pub action: Function<'a>,
    }

    // load transitions from lua land to rust land
    let lua_transitions: Table = globals.get("transitions")?;
    let mut transitions = vec![];
    lua_transitions.for_each::<Value, Table>(|_,v| {
        let name: String = v.get("name")?;
        let guard: Function = v.get("guard")?;
        let action: Function = v.get("action")?;
        transitions.push(Transition {
            name,
            guard,
            action,
        });
        Ok(())
    })?;

    let t = now();
    for _ in 0..1000000 {
        for t in &transitions {
            let r: bool = t.guard.call(())?;
            if r {
//                println!("Took transition: {}", v.get::<_, String>("name")?);
                t.action.call(())?;
            }
        }
    }
    let passed = now() - t;
    println!("Rust evaluation took: {}ms", passed.as_millis());

    // Fetch outputs from lua to rust.

    // "scratch" is an unstructured lua table, can only get as json
    let scratch: Value = globals.get("scratch")?;
    println!("scratch is {}", serde_json::to_string(&scratch).map_err(Error::external)?);

    // robot state is a rust struct.
    let robot1: Robot = lua.from_value(globals.get("robot1")?).map_err(Error::external)?;
    let robot2: Robot = lua.from_value(globals.get("robot2")?).map_err(Error::external)?;

    println!("robot1: {:?}", robot1);
    println!("robot2: {:?}", robot2);

    Ok(())
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

pub fn now() -> std::time::Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
}
