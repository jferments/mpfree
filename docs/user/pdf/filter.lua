-- Pandoc filter for the PDF: the manual's boxes, banners and screenshots
-- become calls to functions defined in manual.typ.

local function typst(inlines)
  local doc = pandoc.Pandoc({ pandoc.Plain(inlines) })
  return (pandoc.write(doc, "typst"):gsub("%s+$", ""))
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
