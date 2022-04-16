#!/usr/bin/env python

from setuptools import setup
from setuptools_rust import Binding, RustExtension


with open('requirements.txt', 'r', encoding='utf-8') as f:
    install_requires = f.read().splitlines()

with open('requirements-dev.txt', 'r', encoding='utf-8') as f:
    setup_requires = f.read().splitlines()

with open('README.md', 'r', encoding='utf-8') as f:
    readme = f.read()


setup(
    name='discord-ext-native-voice',
    author='Rapptz',
    url='https://github.com/Rapptz/discord-ext-native-voice',

    version='0.1.0',
    long_description=readme,
    long_description_content_type='text/markdown',

    packages=['discord.ext.native_voice'],
    include_package_data=True,
    zip_safe=False,
    rust_extensions=[RustExtension('discord.ext.native_voice._native_voice', binding=Binding.PyO3)],
    
    install_requires=install_requires,
    setup_requires=setup_requires,
    python_requires='>=3.8',

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
)
