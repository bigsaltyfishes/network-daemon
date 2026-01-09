import json
import xml.etree.ElementTree as ET
tree = ET.parse('/etc/regdomain.xml')
root = tree.getroot()

regdomain = []
for rd in root.iter('rd'):
    try:
        regdomain.append(rd.attrib['id'])
    except KeyError:
        pass

area = []
for c in root.iter('country'):
    try:
        area.append(c.attrib['id'])
    except KeyError:
        pass

result = {
    "regdomain": regdomain,
    "area": area
}

with open('regdomain.json', 'w') as f:
    json.dump(result, f, indent=4)