-- lua1 (Daraja L1.1, COMPAT_L1_PLAN 6 step 1): strings, tables, integers and floats, and one large allocation that is
-- freed again, so brk and mmap are both used. Deterministic: no time, no randomness, no addresses printed. Its stdout,
-- stderr and exit code must be byte-equal on Linux and AfriSovera.
local t = {}
for i = 1, 1000 do t[i] = i * i end
local s = 0
for _, v in ipairs(t) do s = s + v end
print("sum of squares 1..1000", s)

local words = {}
for w in ("the quick brown fox jumps over the lazy dog"):gmatch("%a+") do words[#words + 1] = w end
table.sort(words)
print("sorted", table.concat(words, " "))
print("upper", ("habari dunia"):upper(), "rep", ("ab"):rep(3, "-"))
print("format", string.format("%5.3f|%x|%q", math.pi, 48879, "a\nb"))
print("integer division and floats", 7 // 2, 7 / 2, 2 ^ 10, math.maxinteger, math.tointeger(3.0))

-- a 4 MiB string, then dropped and collected: the allocator grows and gives memory back
local big = string.rep("x", 4 * 1024 * 1024)
print("big", #big, big:sub(1, 3))
big = nil
collectgarbage("collect")
print("after collect, a small table still works", #t)

local ok, err = pcall(function() error("deliberate") end)
print("pcall", ok, (err:gsub("^.-:%d+: ", "")))
os.exit(3)
