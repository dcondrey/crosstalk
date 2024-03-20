from setuptools import setup, find_packages

setup(
    name='crosstalk',
    version='0.1.0',
    author='David Condrey',
    author_email='davidcondrey@protonmail.com',
    description='A package for exploring problems iteratively using OpenAI and local summarization.',
    packages=find_packages(),
    install_requires=[
        'openai',
        'transformers',
    ],
    entry_points={
        'console_scripts': [
            'crosstalk=crosstalk.crosstalk:main',
        ],
    },
)

