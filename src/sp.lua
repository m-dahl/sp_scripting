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
