"""Tests for slurm_term.sbatch_parser — parse .sbatch files into form state."""

from __future__ import annotations

import textwrap
import tempfile
from pathlib import Path

import pytest

from slurm_term.sbatch_parser import parse_sbatch_file, parse_sbatch_text


# ---------------------------------------------------------------------------
# parse_sbatch_text — core parser
# ---------------------------------------------------------------------------

class TestParseSbatchText:
    def test_basic_directives(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=my_job
            #SBATCH --partition=gpu
            #SBATCH --time=08:00:00
            #SBATCH --nodes=2
            #SBATCH --ntasks-per-node=4
            #SBATCH --cpus-per-task=8
            #SBATCH --mem=32G
            #SBATCH --output=%x-%j.out
            #SBATCH --error=%x-%j.err

            python train.py
        """)
        state = parse_sbatch_text(script)
        assert state["mode"] == "sbatch"
        assert state["name"] == "my_job"
        assert state["partition"] == "gpu"
        assert state["time"] == "08:00:00"
        assert state["nodes"] == "2"
        assert state["ntasks"] == "4"
        assert state["cpus"] == "8"
        assert state["memory"] == "32G"
        assert state["output"] == "%x-%j.out"
        assert state["error"] == "%x-%j.err"

    def test_gpu_gres(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --gres=gpu:a100:2

            nvidia-smi
        """)
        state = parse_sbatch_text(script)
        assert state["gpus"] == "a100:2"

    def test_gpu_gres_simple(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --gres=gpu:1
        """)
        state = parse_sbatch_text(script)
        assert state["gpus"] == "1"

    def test_module_loads(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=test

            module load cuda/12.0
            module load python/3.11

            python train.py
        """)
        state = parse_sbatch_text(script)
        assert state["modules"] == "cuda/12.0\npython/3.11"

    def test_export_vars(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=test

            export CUDA_VISIBLE_DEVICES=0
            export OMP_NUM_THREADS=8

            ./run.sh
        """)
        state = parse_sbatch_text(script)
        assert state["env"] == "CUDA_VISIBLE_DEVICES=0\nOMP_NUM_THREADS=8"

    def test_init_commands(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=test

            cd /home/user/project
            source venv/bin/activate
            python train.py --lr 0.001
        """)
        state = parse_sbatch_text(script)
        lines = state["init"].splitlines()
        assert "cd /home/user/project" in lines
        assert "source venv/bin/activate" in lines
        assert "python train.py --lr 0.001" in lines

    def test_comments_ignored(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            # This is a comment
            #SBATCH --job-name=test
            # Another comment

            echo hello
        """)
        state = parse_sbatch_text(script)
        assert state["name"] == "test"
        assert "comment" not in state["init"].lower()

    def test_short_flags(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH -J short_job
            #SBATCH -p batch
            #SBATCH -t 01:00:00
            #SBATCH -N 1
            #SBATCH -c 4
        """)
        state = parse_sbatch_text(script)
        assert state["name"] == "short_job"
        assert state["partition"] == "batch"
        assert state["time"] == "01:00:00"
        assert state["nodes"] == "1"
        assert state["cpus"] == "4"

    def test_unknown_directives_stored_as_extra(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=test
            #SBATCH --mail-type=END
            #SBATCH --mail-user=user@example.com
            #SBATCH --account=myproject

            echo done
        """)
        state = parse_sbatch_text(script)
        assert state["name"] == "test"
        extra = state.get("extra_directives", {})
        assert extra["mail-type"] == "END"
        assert extra["mail-user"] == "user@example.com"
        assert extra["account"] == "myproject"

    def test_empty_script(self):
        state = parse_sbatch_text("")
        assert state["mode"] == "sbatch"
        assert state["name"] == ""
        assert state["init"] == ""

    def test_full_realistic_script(self):
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=gpu-training
            #SBATCH --partition=gpu
            #SBATCH --time=08:00:00
            #SBATCH --nodes=1
            #SBATCH --ntasks-per-node=1
            #SBATCH --cpus-per-task=4
            #SBATCH --mem=32G
            #SBATCH --gres=gpu:1
            #SBATCH --output=%x-%j.out
            #SBATCH --error=%x-%j.err

            module load cuda
            module load python

            export CUDA_VISIBLE_DEVICES=0

            cd /home/user/project
            source .venv/bin/activate
            python train.py --epochs 100 --batch-size 32
        """)
        state = parse_sbatch_text(script)
        assert state["name"] == "gpu-training"
        assert state["partition"] == "gpu"
        assert state["time"] == "08:00:00"
        assert state["nodes"] == "1"
        assert state["ntasks"] == "1"
        assert state["cpus"] == "4"
        assert state["memory"] == "32G"
        assert state["gpus"] == "1"
        assert state["output"] == "%x-%j.out"
        assert state["error"] == "%x-%j.err"
        assert "cuda" in state["modules"]
        assert "python" in state["modules"]
        assert "CUDA_VISIBLE_DEVICES=0" in state["env"]
        assert "python train.py --epochs 100 --batch-size 32" in state["init"]

    def test_no_extra_directives_key_when_empty(self):
        """When there are no unknown directives, extra_directives should be absent."""
        script = textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=test
        """)
        state = parse_sbatch_text(script)
        assert "extra_directives" not in state


# ---------------------------------------------------------------------------
# parse_sbatch_file — file loading
# ---------------------------------------------------------------------------

class TestParseSbatchFile:
    def test_reads_file(self, tmp_path: Path):
        sbatch = tmp_path / "job.sbatch"
        sbatch.write_text(textwrap.dedent("""\
            #!/bin/bash
            #SBATCH --job-name=file_test
            #SBATCH --time=01:00:00

            echo hello
        """))
        state = parse_sbatch_file(str(sbatch))
        assert state["name"] == "file_test"
        assert state["time"] == "01:00:00"
        assert "echo hello" in state["init"]

    def test_file_not_found(self):
        with pytest.raises(FileNotFoundError):
            parse_sbatch_file("/nonexistent/path/to.sbatch")

    def test_path_object(self, tmp_path: Path):
        sbatch = tmp_path / "test.sbatch"
        sbatch.write_text("#SBATCH --job-name=pathobj\n")
        state = parse_sbatch_file(sbatch)
        assert state["name"] == "pathobj"
