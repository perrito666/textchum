; HCL: Terraform, Terragrunt, Packer and Nomad speak it, with different
; block keywords each. Adapted from nvim-treesitter's queries/hcl and
; queries/terraform highlights (Apache-2.0), with the capture names the
; editor's themes know and without the predicates only Neovim runs.

[
  "!" "*" "/" "%" "+" "-"
  ">" ">=" "<" "<=" "==" "!="
  "&&" "||"
] @operator

["{" "}" "[" "]" "(" ")"] @punctuation.bracket

["." ".*" "," "[*]"] @punctuation.delimiter

[(ellipsis) "?" "=>"] @punctuation.special

["for" "endfor" "in"] @repeat

["if" "else" "endif"] @conditional

[
  (quoted_template_start)
  (quoted_template_end)
  (template_literal)
] @string

[(heredoc_identifier) (heredoc_start)] @punctuation.delimiter

[
  (template_interpolation_start)
  (template_interpolation_end)
  (template_directive_start)
  (template_directive_end)
  (strip_marker)
] @punctuation.special

(numeric_lit) @number
(bool_lit) @boolean
(null_lit) @constant.builtin
(comment) @comment

(identifier) @variable

; Terraform's type names, wherever they appear. Early so that a
; function or attribute of the same name — map(...), list = ... — is
; coloured as what it is by the patterns after this one.
((identifier) @type.builtin
  (#any-of? @type.builtin
    "bool" "string" "number" "object" "tuple" "list" "map" "set" "any"))

; The word that opens a block at the top level is the dialect's own
; keyword — resource, variable, module; include, dependency, inputs;
; source, build; job — so no list of them is needed. A block nested in
; one is its kind.
(body (block (identifier) @keyword))
(body (block (body (block (identifier) @type))))

(function_call (identifier) @function)

(attribute (identifier) @property)

(object_elem
  key: (expression (variable_expr (identifier) @property)))

; var.name, local.name, data.kind.name, module.name: the root of a
; reference is a builtin, what follows it a member.
(expression
  (variable_expr (identifier) @variable.builtin)
  (get_attr (identifier) @property))
