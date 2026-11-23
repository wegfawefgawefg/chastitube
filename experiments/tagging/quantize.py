"""Shrink the ONNX encoder: int8 weights (dynamic quantization). One-time, dev side only."""
import sys
from onnxruntime.quantization import quantize_dynamic, QuantType
src, dst = sys.argv[1], sys.argv[2]
quantize_dynamic(src, dst, weight_type=QuantType.QInt8, per_channel=True,
                 extra_options={"MatMulConstBOnly": True})
