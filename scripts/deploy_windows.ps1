if($env:APPVEYOR_REPO_TAG -eq 'true') {
    Write-Output ("Deploying " + $env:APPVEYOR_REPO_TAG_NAME + " to PyPI...")
    & "C:/Python36/python.exe" -m pip install twine
    & "C:/Python36/python.exe" -m twine upload --skip-existing wheelhouse/*.whl
} else {
    Write-Output "Nothing to deploy"
}
