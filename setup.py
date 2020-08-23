#!/usr/bin/env python
import sys
from setuptools import setup
from setuptools_rust import RustExtension

setup_requires = [
    'setuptools-rust>=0.11.1,<0.12',
    'wheel',
]

setup(
    name='discord-ext-native-voice',
    version='0.1.0',
    classifiers=[
        "License :: OSI Approved :: Apache 2.0",
        "Development Status :: 3 - Alpha",
        "Intended Audience :: Developers",
        "Programming Language :: Python",
        "Programming Language :: Rust",
        "Operating System :: POSIX",
        "Operating System :: Windows",
        "Operating System :: MacOS",
    ],
    packages=['discord.ext.native_voice'],
    rust_extensions=[RustExtension('discord.ext.native_voice._native_voice')],
    install_requires=[],
    setup_requires=setup_requires,
    include_package_data=True,
    zip_safe=False,
)
