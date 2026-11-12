-- Pandoc filter for the PDF: the manual's boxes, banners, screenshots and
-- tables become calls to functions defined in lib.typ.

local function typst_blocks(blocks)
  return (pandoc.write(pandoc.Pandoc(blocks), "typst"):gsub("%s+$", ""))
end

local function typst(inlines)
  return typst_blocks({ pandoc.Plain(inlines) })
end

function Div(el)
  local kind
  if el.classes:includes("mpfbox") then
    kind = "mpfbox"
  elseif el.classes:includes("mpfstatus") then
    kind = "mpfstatus"
  else
    return nil
  end
  local variant = el.classes[2] or ""
  local out = { pandoc.RawBlock("typst", "#" .. kind .. "(\"" .. variant .. "\")[") }
  for _, b in ipairs(el.content) do table.insert(out, b) end
  table.insert(out, pandoc.RawBlock("typst", "]"))
  return out
end

function Para(el)
  if #el.content == 1 and el.content[1].t == "Image" then
    local img = el.content[1]
    return pandoc.RawBlock("typst",
      "#shot(\"" .. img.src .. "\")[" .. typst(img.caption) .. "]")
  end
end

-- A table: #mtable(columns, (header cells), cells...), so lib.typ styles
-- every table the same way and repeats the header on a new page.
local function cell(c)
  return "[" .. typst_blocks(c.contents) .. "]"
end

function Table(el)
  local cols = #el.colspecs
  local head = {}
  if #el.head.rows > 0 then
    for _, c in ipairs(el.head.rows[1].cells) do table.insert(head, cell(c)) end
  end
  while #head < cols do table.insert(head, "[]") end
  -- Each column's share of the width follows its longest text (within
  -- limits), so a column of button names stays narrow and the
  -- descriptions get the room.
  local longest = {}
  for i = 1, cols do longest[i] = 4 end
  local function measure(row)
    for i, c in ipairs(row.cells) do
      local n = utf8.len(pandoc.utils.stringify(c.contents)) or 0
      if i <= cols and n > longest[i] then longest[i] = n end
    end
  end
  if #el.head.rows > 0 then measure(el.head.rows[1]) end
  local cells = {}
  for _, body in ipairs(el.bodies) do
    for _, row in ipairs(body.body) do
      measure(row)
      for _, c in ipairs(row.cells) do table.insert(cells, cell(c)) end
    end
  end
  local widths = {}
  for i = 1, cols do
    local w = math.max(10, math.min(longest[i], 60))
    widths[i] = string.format("%.1ffr", w)
  end
  -- A first column of short labels takes the width it needs, unbroken.
  if longest[1] <= 24 then widths[1] = "auto" end
  return pandoc.RawBlock("typst", "#mtable((" .. table.concat(widths, ", ") .. ",), (" ..
    table.concat(head, ", ") .. ",), " .. table.concat(cells, ", ") .. ")")
end
