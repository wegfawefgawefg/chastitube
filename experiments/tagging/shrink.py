"""Smaller copies of the ONNX bundle that keep all of Laya. One-time, dev side only.

  shrink.py fp16 <src dir> <dst dir>       weights stored as fp16, cast back to fp32 at load
  shrink.py int8w <src dir> <dst dir>      MatMul weights 8-bit in blocks of 32, math in fp32
  shrink.py int4w <src dir> <dst dir>      same, 4-bit
"""
import os, shutil, sys
import numpy as np, onnx
from onnx import helper, numpy_helper, TensorProto

def fp16_storage(m):
    g = m.graph
    keep, casts = [], []
    for init in g.initializer:
        if init.data_type == TensorProto.FLOAT and np.prod(init.dims) >= 1024:
            arr = numpy_helper.to_array(init).astype(np.float16)
            keep.append(numpy_helper.from_array(arr, init.name + "__fp16"))
            casts.append(helper.make_node("Cast", [init.name + "__fp16"], [init.name], to=TensorProto.FLOAT))
        else:
            keep.append(init)
    del g.initializer[:]
    g.initializer.extend(keep)
    nodes = casts + list(g.node)
    del g.node[:]
    g.node.extend(nodes)
    return m

def nbits(path, bits):
    from onnxruntime.quantization.matmul_nbits_quantizer import MatMulNBitsQuantizer
    q = MatMulNBitsQuantizer(onnx.load(path), bits=bits, block_size=32, is_symmetric=True)
    q.process()
    return q.model.model

mode, src, dst = sys.argv[1:4]
os.makedirs(dst, exist_ok=True)
for f in ("tokenizer.json", "rl_agent_config.json"):
    shutil.copy(os.path.join(src, f), dst)
for f in ("encoder.onnx", "head.onnx"):
    p = os.path.join(src, f)
    m = fp16_storage(onnx.load(p)) if mode == "fp16" else nbits(p, 8 if mode == "int8w" else 4)
    onnx.save(m, os.path.join(dst, f))
    print(f, round(os.path.getsize(os.path.join(dst, f)) / 1e6), "MB", flush=True)
