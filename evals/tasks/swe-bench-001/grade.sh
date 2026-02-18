#!/usr/bin/env bash
# Grading script for SWE-bench sympy__sympy-21379
# 1. Verify the reproduction script no longer raises PolynomialError
# 2. Apply the official test patch (adds regression test)
# 3. Run FAIL_TO_PASS test (test_Mod) — must pass
# 4. Run a sample of PASS_TO_PASS tests — must not regress
set -euo pipefail
cd "$EVAL_WORKSPACE"

echo "=== Step 1: Verify reproduction script ==="
python3 -c "
from sympy import *
x_r, y_r = symbols('x_r y_r', real=True)
z = symbols('z')
expr = exp(sinh(Piecewise((x_r, y_r > x_r), (y_r, True)) / z))
expr.subs({1: 1.0})
print('Reproduction script passed — no PolynomialError')
"

echo "=== Step 2: Apply test patch ==="
# The official test patch adds regression tests to test_Mod in test_arit.py
cat > /tmp/test_patch.diff << 'PATCH_EOF'
diff --git a/sympy/core/tests/test_arit.py b/sympy/core/tests/test_arit.py
--- a/sympy/core/tests/test_arit.py
+++ b/sympy/core/tests/test_arit.py
@@ -1913,6 +1913,16 @@ def test_Mod():
     assert Mod(x, y).rewrite(floor) == x - y*floor(x/y)
     assert ((x - Mod(x, y))/y).rewrite(floor) == floor(x/y)

+    # issue 21373
+    from sympy.functions.elementary.trigonometric import sinh
+    from sympy.functions.elementary.piecewise import Piecewise
+
+    x_r, y_r = symbols('x_r y_r', real=True)
+    (Piecewise((x_r, y_r > x_r), (y_r, True)) / z) % 1
+    expr = exp(sinh(Piecewise((x_r, y_r > x_r), (y_r, True)) / z))
+    expr.subs({1: 1.0})
+    sinh(Piecewise((x_r, y_r > x_r), (y_r, True)) * z ** -1.0).is_zero
+

 def test_Mod_Pow():
     # modular exponentiation
PATCH_EOF

git apply /tmp/test_patch.diff
echo "Test patch applied successfully"

echo "=== Step 3: Run FAIL_TO_PASS test (test_Mod) ==="
python3 -m pytest sympy/core/tests/test_arit.py::test_Mod -x -q 2>&1 || {
    echo "FAIL: test_Mod did not pass"
    exit 1
}

echo "=== Step 4: Run PASS_TO_PASS sample tests ==="
# Run a small representative sample to check for regressions
python3 -m pytest \
    sympy/core/tests/test_arit.py::test_bug1 \
    sympy/core/tests/test_arit.py::test_Symbol \
    sympy/core/tests/test_arit.py::test_div \
    sympy/core/tests/test_arit.py::test_pow \
    sympy/core/tests/test_arit.py::test_Mod_Pow \
    sympy/core/tests/test_arit.py::test_Mod_is_integer \
    sympy/core/tests/test_arit.py::test_Mod_is_nonposneg \
    sympy/core/tests/test_arit.py::test_Add_is_zero \
    -x -q 2>&1 || {
    echo "FAIL: PASS_TO_PASS regression detected"
    exit 1
}

echo "=== ALL CHECKS PASSED ==="
exit 0
