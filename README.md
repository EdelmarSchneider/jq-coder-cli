# jqc

**Translate natural-language requests (English + Brazilian Portuguese) into
executable `jq` filters — 100% offline, one binary.** For the JSON you can't
send to a cloud API: production payloads, logs with PII, anything under NDA.

`jqc` is the standalone CLI distribution of
[**jq-coder-0.6B**](https://huggingface.co/DominuZ/jq-coder-0.6B), a 0.6B
fine-tune that runs on your own machine — CPU-only is fine. It embeds
inference (llama.cpp) and the filter executor in a single native binary; the
model weights are downloaded from Hugging Face on first use and cached
locally after that. Evaluation numbers and methodology are on the model card
and on [**jq-bench**](https://huggingface.co/datasets/DominuZ/jq-bench), the
benchmark published alongside it.

## Install

Download a binary from the [Releases page](../../releases). Pick the one
that matches your platform:

| Platform | Acceleration | Binary |
|---|---|---|
| Windows x64 | CPU only | `jqc-windows-x64-cpu.exe` |
| Windows x64 | Vulkan (GPU, with CPU fallback) | `jqc-windows-x64-vulkan.exe` |
| Linux x64 | CPU only | `jqc-linux-x64-cpu` |
| Linux x64 | Vulkan (GPU, with CPU fallback) | `jqc-linux-x64-vulkan` |
| macOS arm64 (Apple Silicon) | Metal (built in) | `jqc-macos-arm64` |

Each binary ships with a `.sha256` file in the same release — verify before
running. On Linux/macOS remember to `chmod +x` the downloaded file.

## Quickstart

```bash
jqc "get the id of every order" orders.json
```

`jqc` prints the generated filter to stderr (`filtro: ...`) and the execution
result to stdout, so the result alone is pipeable.

## Examples

Every example below is a real transcript, verified by execution against this
`orders.json`:

```json
{"orders": [{"id": 1, "status": "done", "total": 120.5},
            {"id": 2, "status": "pending", "total": 40.0}]}
```

### Extract a field from every array element

```console
$ jqc "get the id of every order" orders.json
filtro: .orders[] | .id
1
2
```

### Filter objects by a field value

```console
$ jqc "keep only the orders whose status is done" orders.json
filtro: [.orders[] | select(.status == "done")]
[{"id":1,"status":"done","total":120.5}]
```

### Aggregate values

```console
$ jqc "sum the total of all orders" orders.json
filtro: [.orders[].total] | add
160.5
```

### Delete a field everywhere

```console
$ jqc "remove the total field from every order" orders.json
filtro: .orders |= map(del(.total))
{"orders":[{"id":1,"status":"done"},{"id":2,"status":"pending"}]}
```

## Exemplos em português

The model understands Brazilian Portuguese natively — same binary, no flag
needed. The same examples, asked in Portuguese:

### Extrair um campo de cada elemento

```console
$ jqc "pegue o id de cada pedido" orders.json
filtro: .orders[] | .id
1
2
```

### Somar valores

```console
$ jqc "some o total de todos os pedidos" orders.json
filtro: [.orders[].total] | add
160.5
```

### Remover um campo

```console
$ jqc "remova o campo total de cada pedido" orders.json
filtro: .orders |= map(del(.total))
{"orders":[{"id":1,"status":"done"},{"id":2,"status":"pending"}]}
```

### Write the result back into the file

```bash
jqc "remove the total field from every order" orders.json --write
```

Shows a diff of what would change and asks `write changes? [y/N]` before touching
anything. The previous version is kept as `orders.json.bak`; the write is atomic
(temp file + rename). `--yes` skips the question (for scripts).

### Interactive session

```bash
jqc orders.json
```

Loads the model once and opens a prompt. Type requests in plain English (or
Brazilian Portuguese); results are previews until you apply them:

```
jqc> remove the total field from every order
filter: del(.orders[].total)
{...preview...}
jqc> :a        # apply result to the working buffer
jqc> sort orders by id, descending
jqc> :a
jqc> :w        # diff against the file + write changes? [y/N]
jqc> :q
```

`:d` undoes the last apply, `:?` shows help. The file on disk only changes at `:w`.

## Usage

```
jqc <request> <file.json> [flags]
```

- `<file.json>` — pass `-` to read the JSON document from stdin instead of a
  file.
- `--so-filtro` — print only the generated filter (stdout) and exit; do not
  execute it.
- `--offline` — never touch the network; fail instead of downloading if the
  model is not already cached.
- `--device auto|cpu` — inference device. `auto` (default) uses GPU
  acceleration when the binary was built with it (Vulkan/Metal), falling
  back to CPU automatically if the GPU is unavailable; `cpu` forces CPU.
- `--modelo <revision>` — use a specific Hugging Face revision of the model
  instead of the pinned default.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Model error: the model produced no usable filter, the executor rejected the generated filter, or execution timed out. |
| 2 | Environment error: the input file could not be read, the JSON was invalid, the model download failed, `--offline` was given but nothing is cached, or `--modelo` named an invalid revision. |

## First use

The first run downloads the model — a ~640 MB Q8_0 GGUF, pinned to a fixed
revision — into your OS's cache directory:

| OS | Cache path |
|---|---|
| Windows | `%LOCALAPPDATA%\jq-coder` |
| Linux | `~/.cache/jq-coder` |
| macOS | `~/Library/Caches/jq-coder` |

After that first download, `jqc` never touches the network again (and
`--offline` makes that a hard guarantee, failing instead of downloading).

## Honest limitations

- **Long compositions fail** (roughly two-thirds of jq-bench's human slice):
  `all(...)`, array subtraction (`. - [...]`), `to_entries` with object
  reconstruction + `tonumber`, compound merges with `del`. The model
  interpolates between compositions it saw in training; requests far from
  that come out wrong or hallucinated.
- Aggregations under nested fields can come out as listings instead of sums.
- **Compact JSON (no spaces after `:`/`,`) is out-of-distribution** for some
  compositions: training data was generated with `json.dumps`-style spacing,
  and a small, tightly-packed document can occasionally push a `del`-style
  request into a hallucinated filter that fails at runtime. Spaced JSON
  (the format `jq`/`json.dumps` produce by default) does not show this.
- The request must mention fields by their real names in the JSON — the CLI
  always includes a (possibly pruned) sample of your document in the prompt,
  since the model was trained to depend on it.
- English and Brazilian Portuguese only.
- It generates **filters**, not shell invocations: no `-r`/`-s`/`--arg`
  handling.
- The embedded executor is [jaq](https://github.com/01mf02/jaq), not the
  official `jq` binary (see [ADR-001 in `docs/DECISIONS.md`](docs/DECISIONS.md)
  for the measurement behind that choice). One known divergence: `join(sep)`
  over an array containing `null` — `jq` turns `null` into `""`
  (`["a",null] | join(",")` → `"a,"`), jaq prints the literal word `"null"`
  (`"a,null"`). Measured at 0% on jq-bench's human slice and 0.5% on the
  in-distribution sample.

Always review generated filters before running them against data you care
about — especially destructive ones (`del`, assignments).

## Licenses and attribution

- **Code**: MIT (Copyright Edelmar Schneider).
- **Model weights**: CC BY 4.0. Commercial use welcome. If you share the
  weights or a derivative (re-hosts, quantizations, further fine-tunes),
  credit **Edelmar Schneider**, link back to the
  [model page](https://huggingface.co/DominuZ/jq-coder-0.6B), and indicate
  changes.

If this tool, the model, or the benchmark is useful in your work, please
cite:

```bibtex
@misc{jqcoder2026,
  title   = {jq-coder: a 0.6B offline natural-language-to-jq model and jq-bench},
  author  = {Edelmar Schneider},
  year    = {2026},
  url     = {https://huggingface.co/DominuZ/jq-coder-0.6B}
}
```
