-- redefines some agv positions from ARIAC to axelline
-- (just to avoid rewriting control logic)
location_to_name = {
   [0] = 'pou',      -- conveyor
   [1] = 'waiting',  -- station 1
   [2] = 'wp18',     -- station 2
   [3] = 'kitting',  -- depot
   [99] = 'unknown', -- moving
}
name_to_location = table_invert(location_to_name)

-- set up SP communication via redis.
sp.add_input { lua_name = 'agv1.state', redis_key = 'agv1/state' }
sp.add_input { lua_name = 'agv2.state', redis_key = 'agv2/state' }
sp.add_input { lua_name = 'agv3.state', redis_key = 'agv3/state' }

sp.add_output { lua_name = 'agv1.goal', redis_key = 'agv1/goal', ttl = 20 }
sp.add_output { lua_name = 'agv2.goal', redis_key = 'agv2/goal', ttl = 20 }
sp.add_output { lua_name = 'agv3.goal', redis_key = 'agv3/goal', ttl = 20 }

-- setup initial states if needed
agv1 = {
   position = "waiting",
   has_kit = true,
}
agv2 = {
   position = "pou",
   has_kit = true,
}
agv3 = {
   position = "kitting",
   has_kit = false,
}
roles = {
   waiting = "agv1",
   pou = "agv2",
   kitting = "agv3",
}

-- "pointers"
function kitting()
   return _G[roles.kitting]
end

function waiting()
   return _G[roles.waiting]
end

function pou()
   return _G[roles.pou]
end

-- For demonstration operations just sleep a bit before finishing the
-- ack operations.
dummy_time = 5000

function swap_roles()
   roles = {
      kitting = roles.pou,
      pou = roles.waiting,
      waiting = roles.kitting
   }
end

function axelline_coordinator()
   if not kitting().operation and kitting().position ~= "kitting" then
      local op_name = roles.kitting .. "_goto_kitting"
      sp.add_operation {
         name = op_name,

         -- guard includes waitings position.
         start_guard = function (state) return waiting().position == "wp18" end,
         start_action = function (state)
            state.started_at = sp.now
            kitting().goal = name_to_location['kitting']
            kitting().operation = op_name
         end,

         -- pretend to finish after some time
         finish_guard = function (state)
            return location_to_name[kitting().state.location] == 'kitting'
         end,
         finish_action = function (state)
            kitting().operation = nil
            kitting().position = "kitting"
            sp.remove_operation(op_name)
         end,
      }
   end

   if not kitting().operation and kitting().position == "kitting"
      and not kitting().has_kit then
      local op_name = roles.kitting .. "_kitting_ack"
      sp.add_operation {
         name = op_name,

         start_guard = function (state) return true end,
         start_action = function (state)
            state.started_at = sp.now
            kitting().operation = op_name
         end,

         finish_guard = function (state)
            return sp.now > state.started_at + dummy_time
         end,
         finish_action = function (state)
            kitting().operation = nil
            kitting().has_kit = true
            sp.remove_operation(op_name)
         end,
      }
   end

   if not waiting().operation
      and waiting().position ~= "wp18"
      and waiting().position ~= "waiting" then
      local op_name = roles.waiting .. "_goto_wp18"
      sp.add_operation {
         name = op_name,

         start_guard = function (state) return true end,
         start_action = function (state)
            state.started_at = sp.now
            waiting().goal = name_to_location['wp18']
            waiting().operation = op_name
         end,

         finish_guard = function (state)
            return location_to_name[waiting().state.location] == 'wp18'
         end,
         finish_action = function (state)
            waiting().operation = nil
            waiting().position = "wp18"
            sp.remove_operation(op_name)
         end,
      }
   end

   if not waiting().operation
      and waiting().position == "wp18" then
      local op_name = roles.waiting .. "_goto_waiting"
      sp.add_operation {
         name = op_name,

         -- guard includes kitting position. check
         -- wp17 etc... for simplicity here we
         -- check that kitting has moved all the
         -- way to the kitting wp
         start_guard = function (state) return kitting().position == "kitting" end,
         start_action = function (state)
            state.started_at = sp.now
            waiting().goal = name_to_location['waiting']
            waiting().operation = op_name
         end,

         finish_guard = function (state)
            return location_to_name[waiting().state.location] == 'waiting'
         end,
         finish_action = function (state)
            waiting().operation = nil
            waiting().position = "waiting"
            sp.remove_operation(op_name)
         end,
      }
   end

   if not pou().operation then
      local op_name = roles.pou .. "_pou_ack"
      sp.add_operation {
         name = op_name,

         start_guard = function (state) return true end,
         start_action = function (state)
            state.started_at = sp.now
            pou().operation = op_name
         end,

         finish_guard = function (state)
            return waiting().position == "waiting" and kitting().has_kit and
               sp.now > state.started_at + dummy_time
         end,
         finish_action = function (state)
            pou().operation = nil
            pou().has_kit = false -- remove the kit.
            sp.remove_operation(op_name)

            -- when we finish we also want to swap the roles.
            swap_roles()
         end,
      }
   end
end

-- adds this function to the sp ticker.
sp.add_function("axelline_coordinator", axelline_coordinator)
