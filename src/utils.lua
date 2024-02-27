function table_invert(t)
   local s={}
   for k,v in pairs(t) do
     s[v]=k
   end
   return s
end

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
