-- Boot, press keys, screenshot chosen frames. Env: KEYS (file of "first last hexmask" lines;
-- 02 Start, 04 A, 08 B, 10/20/40/80 X up/right/down/left), SHOTS (comma list of frames), OUT.
local out = os.getenv("OUT")
local shots, last = {}, 0
for n in (os.getenv("SHOTS") or "300"):gmatch("%d+") do shots[tonumber(n)] = true; last = math.max(last, tonumber(n)) end
local bits = { [0x02] = "start", [0x04] = "a", [0x08] = "b", [0x10] = "up", [0x20] = "right", [0x40] = "down", [0x80] = "left", [0x100]="up2", [0x200]="right2", [0x400]="down2", [0x800]="left2" }
local presses = {}
local kf = os.getenv("KEYS")
if kf and kf ~= "" then
  for line in io.lines(kf) do
    local a, b, m = line:match("^%s*(%d+)%s+(%d+)%s+(%x+)")
    if a then presses[#presses + 1] = { tonumber(a), tonumber(b), tonumber(m, 16) } end
  end
end
local frames, done = 0, false
emu.addEventCallback(function()
  local input = {}
  for _, p in ipairs(presses) do
    if frames >= p[1] and frames <= p[2] then
      for bit, name in pairs(bits) do if p[3] & bit ~= 0 then input[name] = true end end
    end
  end
  emu.setInput(input, 0)
end, emu.eventType.inputPolled)
emu.addEventCallback(function()
  if done then return end
  frames = frames + 1
  if shots[frames] then
    local f = io.open(string.format("%s/shot-%05d.png", out, frames), "wb") f:write(emu.takeScreenshot()) f:close()
    if os.getenv("RAM") == "1" then
      local t = {}
      for a = 0, 0xffff do t[#t + 1] = string.char(emu.read(a, emu.memType.wsWorkRam)) end
      local g = io.open(string.format("%s/ram-%05d.bin", out, frames), "wb") g:write(table.concat(t)) g:close()
      local p = {}
      for a = 0, 0xff do p[#p + 1] = string.char(emu.read(a, emu.memType.wsPort, false)) end
      local h = io.open(string.format("%s/ports-%05d.bin", out, frames), "wb") h:write(table.concat(p)) h:close()
    end
  end
  if frames >= last then
    local f = io.open(out .. "/done.txt", "w") f:write(tostring(frames)) f:close()
    done = true
    emu.exit(0)
  end
end, emu.eventType.endFrame)
-- An existing play state is a private behavior reference, never an asset source.
local statePath=os.getenv('STATE')
if statePath and statePath~='' then
 local f=assert(io.open(statePath,'rb'));local data=f:read('*a');f:close()
 local loaded=false;local ref=nil
 ref=emu.addMemoryCallback(function()
  if loaded then return end
  loaded=true;local ok=emu.loadSavestate(data);frames=0
  local f=io.open(out..'/state-load.txt','w');f:write(tostring(ok));f:close()
  if not ok then emu.exit(2) end
 end,emu.callbackType.exec,0,0xfffff,emu.cpuType.ws)
 emu.addEventCallback(function()if loaded and ref then emu.removeMemoryCallback(ref,emu.callbackType.exec,0,0xfffff,emu.cpuType.ws);ref=nil end end,emu.eventType.endFrame)
end
-- Optional behavior capture: state-key inventory and per-scanline display ports.
-- This samples the documented debugger API; no emulator implementation is used.
if os.getenv('RASTER')=='1' then
 local dumped=false;local rows={};local seen={};local rasterFrame=-1
 emu.addMemoryCallback(function()
  if frames<last-2 then return end
  local state=emu.getState()
  if not dumped then
   local keys={};for key,value in pairs(state) do keys[#keys+1]=key..'='..tostring(value) end;table.sort(keys)
   local f=io.open(out..'/state-keys.txt','w');f:write(table.concat(keys,'\n'));f:close();dumped=true
  end
  local line=state['ppu.scanline']
  if line==nil then return end
  if rasterFrame~=frames then rows={};seen={};rasterFrame=frames end
  if line>=0 and line<144 and not seen[line] then
   seen[line]=true;local row={};for p=0,255 do row[#row+1]=string.char(emu.read(p,emu.memType.wsPort,false)) end
   rows[line]=table.concat(row)
   if line==143 then
    local f=io.open(string.format('%s/line-ports-%05d.bin',out,frames+1),'wb')
    for y=0,143 do if rows[y] then f:write(rows[y]) end end;f:close()
   end
  end
 end,emu.callbackType.exec,0,0xfffff,emu.cpuType.ws)
end
